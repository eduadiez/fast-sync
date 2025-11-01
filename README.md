# Fast Sync

A simple, high-performance file watcher and transfer tool written in Rust. It consists of two binaries:

- `watcher`: Watches a directory for new/modified files and sends them to a remote server.
- `client`: Receives files and writes them to a destination directory.

## Features
- Efficient file watching using inotify (Linux)
- Zero-copy file transfer with memory-mapped files
- Integrity verification with BLAKE3 checksums
- Detailed latency logging
- Configurable via command-line arguments

## Usage

### Build

```
cargo build --release
```

### Run the client (receiver)

```
./target/release/client \
    --bind-ip 0.0.0.0 \
    --bind-port 5001 \
    --dest-dir /path/to/destination
```

- `--bind-ip`: IP address to bind the server (default: 0.0.0.0)
- `--bind-port`: Port to listen on (default: 5001)
- `--dest-dir`: Directory to store received files (default: /destino)

### Run the watcher (sender)

```
./target/release/watcher \
    --dest-ip 10.0.0.2 \
    --dest-port 5001 \
    --watch-dir /path/to/watch
```

- `--dest-ip`: Destination IP or FQDN (default: 10.0.0.2)
- `--dest-port`: Destination port (default: 5001)
- `--watch-dir`: Directory to watch for new/modified files (default: /origen)

## Dependencies
- [clap](https://crates.io/crates/clap) for argument parsing
- [anyhow](https://crates.io/crates/anyhow) for error handling
- [tokio](https://crates.io/crates/tokio) for async runtime
- [inotify](https://crates.io/crates/inotify) for file system events
- [memmap2](https://crates.io/crates/memmap2) for memory-mapped files
- [blake3](https://crates.io/crates/blake3) for checksums

## License
MIT
