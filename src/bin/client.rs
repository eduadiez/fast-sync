use anyhow::{Context, Result};
use blake3::Hasher;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpSocket},
};

const BIND_IP: &str = "0.0.0.0";
const BIND_PORT: u16 = 5001;
const DEST_DIR: &str = "/destino"; // <-- ajusta

#[tokio::main]
async fn main() -> Result<()> {
    tokio::fs::create_dir_all(DEST_DIR).await.ok();
    let socket = TcpSocket::new_v4()?;
    socket.set_reuseaddr(true)?;
    socket.set_nodelay(true)?;
    socket.bind(SocketAddr::from(([0, 0, 0, 0], BIND_PORT)))?;
    let listener = socket.listen(1)?;
    eprintln!("[*] Escuchando en {}:{}", BIND_IP, BIND_PORT);

    let (mut conn, peer) = listener.accept().await?;
    conn.set_nodelay(true)?;
    eprintln!("[*] Conectado desde {}", peer);

    loop {
        // Header: u16 name_len
        let mut len_buf = [0u8; 2];
        if conn.read_exact(&mut len_buf).await.is_err() {
            eprintln!("[*] Conexión cerrada");
            break;
        }
        let name_len = u16::from_be_bytes(len_buf) as usize;

        // Nombre
        let mut name_bytes = vec![0u8; name_len];
        conn.read_exact(&mut name_bytes).await?;
        let name = String::from_utf8(name_bytes).context("Nombre no UTF-8")?;

        // Tamaño (u64)
        let mut sz_buf = [0u8; 8];
        conn.read_exact(&mut sz_buf).await?;
        let size = u64::from_be_bytes(sz_buf);

        // Checksum esperado (32 bytes)
        let mut chk = [0u8; 32];
        conn.read_exact(&mut chk).await?;

        let dest_path = Path::new(DEST_DIR).join(&name);
        let tmp_path = PathBuf::from(format!("{}.part", dest_path.display()));
        if let Some(parent) = dest_path.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }

        // Recibir datos al archivo temporal
        let mut hasher = Hasher::new();
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

        // Verificar checksum
        let got = hasher.finalize();
        let ok = got.as_bytes() == &chk;
        if !ok {
            let _ = std::fs::remove_file(&tmp_path);
            let _ = conn.write_all(&[0x00]).await;
            eprintln!("[!] Checksum inválido para {}", name);
            continue;
        }

        // Rename atómico
        std::fs::rename(&tmp_path, &dest_path)?;
        conn.write_all(&[0x01]).await?; // ACK OK
        eprintln!("[+] OK {} ({} bytes)", name, size);
    }
    Ok(())
}
