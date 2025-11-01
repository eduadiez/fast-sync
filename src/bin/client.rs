use anyhow::{Context, Result};
use clap::Parser;
use blake3::Hasher;
use std::{
    fs::OpenOptions,
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpSocket,
};


/// File receiver
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Bind IP address
    #[arg(long, default_value = "0.0.0.0")]
    bind_ip: String,

    /// Bind port
    #[arg(long, default_value_t = 5001)]
    bind_port: u16,

    /// Destination directory
    #[arg(long, default_value = "/destino")]
    dest_dir: String,
}


#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let bind_ip = args.bind_ip;
    let bind_port = args.bind_port;
    let dest_dir = args.dest_dir;

    tokio::fs::create_dir_all(&dest_dir).await.ok();
    let socket = TcpSocket::new_v4()?;
    socket.set_reuseaddr(true)?;
    socket.set_nodelay(true)?;
    socket.bind(SocketAddr::new(bind_ip.parse().unwrap(), bind_port))?;
    let listener = socket.listen(1)?;
    eprintln!("[*] Listening on {}:{}", bind_ip, bind_port);

    let (mut conn, peer) = listener.accept().await?;
    conn.set_nodelay(true)?;
    eprintln!("[*] Connected from {}", peer);

    use std::time::Instant;
    loop {
        let total_start = Instant::now();
        // Header: u16 name_len
        let mut len_buf = [0u8; 2];
        if conn.read_exact(&mut len_buf).await.is_err() {
            eprintln!("[*] Connection closed");
            break;
        }
        let name_len = u16::from_be_bytes(len_buf) as usize;

        // Name
        let mut name_bytes = vec![0u8; name_len];
        let name_start = Instant::now();
        conn.read_exact(&mut name_bytes).await?;
        let name = String::from_utf8(name_bytes).context("Name not UTF-8")?;
        let name_end = Instant::now();

        // Size (u64)
        let mut sz_buf = [0u8; 8];
        let size_start = Instant::now();
        conn.read_exact(&mut sz_buf).await?;
        let size = u64::from_be_bytes(sz_buf);
        let size_end = Instant::now();

        // Expected checksum (32 bytes)
        let mut chk = [0u8; 32];
        let chk_start = Instant::now();
        conn.read_exact(&mut chk).await?;
        let chk_end = Instant::now();

        let dest_path = Path::new(&dest_dir).join(&name);
        let tmp_path = PathBuf::from(format!("{}.part", dest_path.display()));
        if let Some(parent) = dest_path.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }

        // Receive data to temporary file
        let mut hasher = Hasher::new();
        let data_start = Instant::now();
        {
            let mut f = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&tmp_path)?;
            let mut remaining = size as i64;
            let mut buf = vec![0u8; 1024 * 1024];
            while remaining > 0 {
                let to_read = buf.len().min(remaining as usize);
                let n = conn.read_exact(&mut buf[..to_read]).await?;
                if n == 0 {
                    break;
                }
                f.write_all(&buf[..n])?;
                hasher.update(&buf[..n]);
                remaining -= n as i64;
            }
            f.flush()?;
        }
        let data_end = Instant::now();

        // Verify checksum
        let verify_start = Instant::now();
        let got = hasher.finalize();
        let ok = got.as_bytes() == &chk;
        let verify_end = Instant::now();
        if !ok {
            let _ = std::fs::remove_file(&tmp_path);
            let _ = conn.write_all(&[0x00]).await;
            eprintln!("[!] Invalid checksum for {}", name);
            continue;
        }

        // Atomic rename
        let rename_start = Instant::now();
        std::fs::rename(&tmp_path, &dest_path)?;
        let rename_end = Instant::now();
        conn.write_all(&[0x01]).await?; // ACK OK
        let total_end = Instant::now();
        eprintln!(
            "[+] OK {} ({} bytes) | Name: {:.2?} | Size: {:.2?} | Checksum: {:.2?} | Data: {:.2?} | Verify: {:.2?} | Rename: {:.2?} | Total: {:.2?}",
            name,
            size,
            name_end.duration_since(name_start),
            size_end.duration_since(size_start),
            chk_end.duration_since(chk_start),
            data_end.duration_since(data_start),
            verify_end.duration_since(verify_start),
            rename_end.duration_since(rename_start),
            total_end.duration_since(total_start)
        );
    }
    Ok(())
}
