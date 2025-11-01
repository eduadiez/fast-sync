use anyhow::{Context, Result};
use clap::Parser;
use blake3::Hasher;
use inotify::{Inotify, WatchMask};
use memmap2::Mmap;
use std::{fs::File, net::SocketAddr, path::Path, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpSocket, TcpStream},
    time::sleep,
};


/// File watcher and sender
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Destination IP or FQDN
    #[arg(long, default_value = "10.0.0.2")]
    dest_ip: String,

    /// Destination port
    #[arg(long, default_value_t = 5001)]
    dest_port: u16,

    /// Directory to watch (recursive)
    #[arg(long, default_value = "/origen")]
    watch_dir: String,
}


#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let dest_ip = args.dest_ip;
    let dest_port = args.dest_port;
    let watch_dir = args.watch_dir;

    let mut conn = connect_persistent(&dest_ip, dest_port).await?;
    eprintln!("[*] Connected to {}:{}", dest_ip, dest_port);

    // inotify: close after write events (recursive)
    let mut inotify = Inotify::init().context("init inotify")?;
    inotify.watches().add(
        &watch_dir,
        WatchMask::CLOSE_WRITE | WatchMask::MOVED_TO | WatchMask::CREATE | WatchMask::ONLYDIR,
    )?;

    let mut buf = [0u8; 4096];
    loop {
        let events = inotify.read_events_blocking(&mut buf)?;
        for ev in events {
            if let Some(name) = ev.name {
                let full = Path::new(&watch_dir).join(name);
                if full.is_file() {
                    // Optional: wait a few milliseconds for safety (some writers close+rename)
                    use std::time::Instant;
                    let event_time = Instant::now();
                    sleep(Duration::from_millis(1)).await;
                    let send_start = Instant::now();
                    if let Err(e) = send_one(&mut conn, &full, Path::new(&watch_dir)).await {
                        eprintln!("[!] Send error: {e}. Retrying...");
                        // Retry with reconnection
                        conn = connect_persistent(&dest_ip, dest_port).await?;
                        send_one(&mut conn, &full, Path::new(&watch_dir)).await?;
                    }
                    let send_end = Instant::now();
                    let event_to_send = send_start.duration_since(event_time);
                    let send_duration = send_end.duration_since(send_start);
                    eprintln!(
                        "[latency] File: {} | Event-to-send: {:.2?} | Send duration: {:.2?}",
                        full.display(),
                        event_to_send,
                        send_duration
                    );
                }
            }
        }
    }
}

async fn connect_persistent(dest_ip: &str, dest_port: u16) -> Result<TcpStream> {
    loop {
        let socket = match TcpSocket::new_v4() {
            Ok(s) => s,
            Err(_e) => {
                sleep(Duration::from_millis(500)).await;
                continue;
            }
        };
        if let Err(_e) = socket.set_nodelay(true) {
            sleep(Duration::from_millis(500)).await;
            continue;
        }
        let addr = SocketAddr::new(dest_ip.parse().unwrap(), dest_port);
        match socket.connect(addr).await {
            Ok(stream) => return Ok(stream),
            Err(_) => sleep(Duration::from_millis(500)).await,
        }
    }
}

async fn send_one(conn: &mut TcpStream, fullpath: &Path, base: &Path) -> Result<()> {
    use std::time::Instant;
    // relative name
    let rel = fullpath.strip_prefix(base).unwrap_or(fullpath);
    let name = rel.to_string_lossy().to_string();

    let file = File::open(fullpath).with_context(|| format!("Open {}", fullpath.display()))?;
    let size = file.metadata()?.len();

    // mmap to read once and with minimal latency
    let mmap = unsafe { Mmap::map(&file)? };
    let mut hasher = Hasher::new();
    hasher.update(&mmap);
    let digest = hasher.finalize();

    // Header
    let name_bytes = name.as_bytes();
    let mut header = Vec::with_capacity(2 + name_bytes.len() + 8 + 32);
    header.extend_from_slice(&(name_bytes.len() as u16).to_be_bytes());
    header.extend_from_slice(name_bytes);
    header.extend_from_slice(&size.to_be_bytes());
    header.extend_from_slice(digest.as_bytes());
    let write_header_start = Instant::now();
    conn.write_all(&header).await?;

    // Data
    let write_data_start = Instant::now();
    conn.write_all(&mmap).await?;

    // ACK
    let mut ack = [0u8; 1];
    conn.read_exact(&mut ack).await?;
    let write_end = Instant::now();
    if ack[0] != 0x01 {
        anyhow::bail!("Destination reported failure receiving {}", name);
    }
    eprintln!(
        "[+] OK {} ({} bytes) | Header: {:.2?} | Data: {:.2?} | Total: {:.2?}",
        name,
        size,
        write_data_start.duration_since(write_header_start),
        write_end.duration_since(write_data_start),
        write_end.duration_since(write_header_start)
    );
    Ok(())
}
