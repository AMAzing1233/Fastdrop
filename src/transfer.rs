// File transfer operations and protocol decision logic

use crate::protocol::{FileChunk, FileList, FileMetadata, TransportProtocol};
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::fs::{self, File};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/* ========== Constants ========== */

/// Size of each chunk for file transfer (64KB)
pub const CHUNK_SIZE: usize = 64 * 1024;

/// Threshold for protocol selection
const MANY_FILES_THRESHOLD: usize = 5;
const SMALL_TOTAL_SIZE_THRESHOLD: u64 = 100 * 1024 * 1024; // 100 MB

/* ========== Protocol Decision ========== */

/// Analyzes files and decides optimal transport protocol
/// 
/// Rules:
/// - QUIC: Many files (>5) OR small total size (<100MB)
///   Benefits: multiplexing, lower latency, parallel streams
/// - TCP: Few files (≤5) AND large total size (≥100MB)
///   Benefits: simpler, reliable, better congestion control for large transfers
pub async fn analyze_files<P: AsRef<Path>>(
    file_paths: &[P],
) -> Result<(TransportProtocol, FileList)> {
    if file_paths.is_empty() {
        anyhow::bail!("No files provided for analysis");
    }

    let mut files = Vec::new();
    let mut total_size = 0u64;

    // Gather metadata for each file
    for path in file_paths {
        let path = path.as_ref();
        
        // Get file metadata
        let metadata = fs::metadata(path)
            .await
            .with_context(|| format!("Failed to read metadata for {:?}", path))?;
        
        if !metadata.is_file() {
            anyhow::bail!("{:?} is not a regular file", path);
        }

        let size = metadata.len();
        total_size += size;

        // Calculate SHA256 hash
        let hash = calculate_file_hash(path).await?;

        let file_meta = FileMetadata {
            name: path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string(),
            size,
            hash: Some(hash),
        };

        files.push(file_meta);
    }

    // Decide protocol based on file count and total size
    let protocol = if file_paths.len() > MANY_FILES_THRESHOLD
        || total_size < SMALL_TOTAL_SIZE_THRESHOLD
    {
        TransportProtocol::Quic
    } else {
        TransportProtocol::Tcp
    };

    let file_list = FileList { files, total_size };

    println!(
        "📊 Analysis: {} files, {} bytes total → Using {:?}",
        file_list.files.len(),
        file_list.total_size,
        protocol
    );

    Ok((protocol, file_list))
}

/* ========== File Hashing ========== */

/// Calculate SHA256 hash of a file
async fn calculate_file_hash(path: &Path) -> Result<[u8; 32]> {
    let mut file = File::open(path)
        .await
        .with_context(|| format!("Failed to open {:?} for hashing", path))?;

    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; CHUNK_SIZE];

    loop {
        let n = file
            .read(&mut buffer)
            .await
            .context("Failed to read file for hashing")?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }

    let result = hasher.finalize();
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&result);
    Ok(hash)
}

/* ========== File Sending ========== */

/// Send a file as chunks
/// Returns async stream of FileChunk
pub async fn send_file<P: AsRef<Path>>(
    path: P,
    file_index: usize,
) -> Result<Vec<FileChunk>> {
    let path = path.as_ref();
    let mut file = File::open(path)
        .await
        .with_context(|| format!("Failed to open {:?} for sending", path))?;

    let file_size = file
        .metadata()
        .await
        .context("Failed to get file metadata")?
        .len();

    // Calculate total chunks
    let total_chunks = (file_size + CHUNK_SIZE as u64 - 1) / CHUNK_SIZE as u64;

    let mut chunks = Vec::new();
    let mut buffer = vec![0u8; CHUNK_SIZE];
    let mut chunk_number = 0u64;

    loop {
        let n = file
            .read(&mut buffer)
            .await
            .context("Failed to read file chunk")?;
        
        if n == 0 {
            break;
        }

        let chunk = FileChunk {
            file_index,
            chunk_number,
            total_chunks,
            data: buffer[..n].to_vec(),
        };

        chunks.push(chunk);
        chunk_number += 1;
    }

    println!(
        "📤 Prepared {} chunks for file {} ({})",
        chunks.len(),
        file_index,
        path.display()
    );

    Ok(chunks)
}

/* ========== File Receiving ========== */

/// Receive file chunks and write to destination
pub struct FileReceiver {
    file: File,
    file_index: usize,
    expected_chunks: u64,
    received_chunks: u64,
    path: PathBuf,
}

impl FileReceiver {
    /// Create a new file receiver
    pub async fn new<P: AsRef<Path>>(
        path: P,
        file_index: usize,
    ) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        
        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .await
                .with_context(|| format!("Failed to create directory {:?}", parent))?;
        }

        let file = File::create(&path)
            .await
            .with_context(|| format!("Failed to create file {:?}", path))?;

        Ok(Self {
            file,
            file_index,
            expected_chunks: 0,
            received_chunks: 0,
            path,
        })
    }

    /// Write a chunk to the file
    pub async fn write_chunk(&mut self, chunk: FileChunk) -> Result<()> {
        // Validate chunk
        if chunk.file_index != self.file_index {
            anyhow::bail!(
                "Chunk file index mismatch: expected {}, got {}",
                self.file_index,
                chunk.file_index
            );
        }

        // Update expected chunks on first chunk
        if self.expected_chunks == 0 {
            self.expected_chunks = chunk.total_chunks;
        }

        // Write data
        self.file
            .write_all(&chunk.data)
            .await
            .context("Failed to write chunk data")?;

        self.received_chunks += 1;

        println!(
            "📥 Received chunk {}/{} for file {}",
            self.received_chunks, self.expected_chunks, self.file_index
        );

        Ok(())
    }

    /// Check if all chunks have been received
    pub fn is_complete(&self) -> bool {
        self.expected_chunks > 0 && self.received_chunks >= self.expected_chunks
    }

    /// Finalize the file and verify hash
    pub async fn finalize(mut self, expected_hash: Option<[u8; 32]>) -> Result<()> {
        // Flush and sync
        self.file.flush().await.context("Failed to flush file")?;
        self.file.sync_all().await.context("Failed to sync file")?;
        drop(self.file);

        // Verify hash if provided
        if let Some(expected_hash) = expected_hash {
            let actual_hash = calculate_file_hash(&self.path).await?;
            if actual_hash != expected_hash {
                anyhow::bail!(
                    "Hash mismatch for {:?}: expected {:x?}, got {:x?}",
                    self.path,
                    expected_hash,
                    actual_hash
                );
            }
            println!("✅ Hash verified for {:?}", self.path);
        }

        println!("✅ File complete: {:?}", self.path);
        Ok(())
    }
}

/* ========== Utility Functions ========== */

/// Format bytes as human-readable string
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_index = 0;

    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }

    format!("{:.2} {}", size, UNITS[unit_index])
}

/// Calculate transfer progress percentage
pub fn calculate_progress(received: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        (received as f64 / total as f64) * 100.0
    }
}
