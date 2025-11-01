use anyhow::{Context, Result};
use blake3::Hasher;
use inotify::{Inotify, WatchMask};
use memmap2::Mmap;
use std::{fs::File, net::SocketAddr, path::{Path, PathBuf}, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpSocket, TcpStream},
    time::sleep,
};

const DEST_IP: &str   = "10.0.0.2"; // <-- IP/FQDN del destino
const DEST_PORT: u16  = 5001;
const WATCH_DIR: &str = "/origen";   // <-- carpeta a vigilar (recursivo)

#[tokio::main]
async fn main() -> Result<()> {
    let mut conn = connect_persistent().await?;
    eprintln!("[*] Conectado a {}:{}", DEST_IP, DEST_PORT);

    // inotify: eventos de cierre tras escritura (recursivo)
    let mut inotify = Inotify::init().context("init inotify")?;
    inotify.add_watch(WATCH_DIR, WatchMask::CLOSE_WRITE | WatchMask::MOVED_TO | WatchMask::CREATE | WatchMask::ONLYDIR)?;
    // Para recursivo: añade subdirectorios (scan inicial)
    add_subdirs(&mut inotify, Path::new(WATCH_DIR))?;

    let mut buf = [0u8; 4096];
    loop {
        let events = inotify.read_events_blocking(&mut buf)?;
        for ev in events {
            if ev.mask.contains(inotify::EventMask::ISDIR) {
                // si aparece nuevo dir, añádelo al watch
                if let Some(name) = ev.name {
                    let p = Path::new(WATCH_DIR).join(name);
                    if p.is_dir() {
                        let _ = inotify.add_watch(&p, WatchMask::CLOSE_WRITE | WatchMask::MOVED_TO | WatchMask::CREATE | WatchMask::ONLYDIR);
                    }
                }
                continue;
            }
            if let Some(name) = ev.name {
                let full = Path::new(WATCH_DIR).join(name);
                if full.is_file() {
                    // Opcional: espera unos milisegundos por seguridad (algunos writers cierran+renombran)
                    sleep(Duration::from_millis(1)).await;
                    if let Err(e) = send_one(&mut conn, &full, Path::new(WATCH_DIR)) .await {
                        eprintln!("[!] Error envío: {e}. Reintentando...");
                        // Reintento con reconexión
                        conn = connect_persistent().await?;
                        send_one(&mut conn, &full, Path::new(WATCH_DIR)).await?;
                    }
                }
            }
        }
    }
}

async fn connect_persistent() -> Result<TcpStream> {
    loop {
        let socket = match TcpSocket::new_v4() {
            Ok(s) => s,
            Err(e) => {
                sleep(Duration::from_millis(500)).await;
                continue;
            }
        };
        if let Err(e) = socket.set_nodelay(true) {
            sleep(Duration::from_millis(500)).await;
            continue;
        }
        let addr = SocketAddr::new(DEST_IP.parse().unwrap(), DEST_PORT);
        match socket.connect(addr).await {
            Ok(stream) => return Ok(stream),
            Err(_) => sleep(Duration::from_millis(500)).await,
        }
    }
}

async fn send_one(conn: &mut TcpStream, fullpath: &Path, base: &Path) -> Result<()> {
    // nombre relativo
    let rel = fullpath.strip_prefix(base).unwrap_or(fullpath);
    let name = rel.to_string_lossy().to_string();

    let file = File::open(fullpath)
        .with_context(|| format!("Abrir {}", fullpath.display()))?;
    let size = file.metadata()?.len();

    // mmap para leer una vez y con mínima latencia
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
    conn.write_all(&header).await?;

    // Datos
    conn.write_all(&mmap).await?;

    // ACK
    let mut ack = [0u8; 1];
    conn.read_exact(&mut ack).await?;
    if ack[0] != 0x01 {
        anyhow::bail!("Destino reportó fallo al recibir {}", name);
    }
    eprintln!("[+] OK {} ({} bytes)", name, size);
    Ok(())
}

fn add_subdirs(inotify: &mut Inotify, root: &Path) -> Result<()> {
    use std::fs;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {


let e = entry?;
            let p = e.path();
            if p.is_dir() {
                let _ = inotify.add_watch(&p, WatchMask::CLOSE_WRITE | WatchMask::MOVED_TO | WatchMask::CREATE | WatchMask::ONLYDIR);
                stack.push(p);
            }
        }
    }
    Ok(())
}