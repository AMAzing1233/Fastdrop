// Protocol definitions and data structures for Fastdrop
// This module contains only types and constants - no logic

use libp2p::{Multiaddr, PeerId};
use serde::{Deserialize, Serialize};
use serde_big_array::BigArray;

/* ========== BLE UUIDs for Discovery ========== */

// QUIC Protocol UUIDs
pub const QUIC_SERVICE_UUID: &str = "12345678-1234-5678-1234-56789ABCDEF0";
pub const QUIC_CHAR_UUID: &str = "ABCDEFAB-CDEF-1234-5678-1234567890AB";

// TCP Protocol UUIDs
pub const TCP_SERVICE_UUID: &str = "87654321-4321-8765-4321-FEDCBA9876543";
pub const TCP_CHAR_UUID: &str = "BAFEDCBA-FEDC-4321-8765-BA0987654321";

/* ========== File Size Limits ========== */

/// Maximum size for a single file (100 MB)
pub const MAX_FILE_SIZE: u64 = 100 * 1024 * 1024;

/// Maximum total transfer size (500 MB)
pub const MAX_TOTAL_SIZE: u64 = 500 * 1024 * 1024;

/// Chunk size for reading files (64 KB)
pub const CHUNK_SIZE: usize = 64 * 1024;

/* ========== Transport Protocol Selection ========== */

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TransportProtocol {
    /// QUIC transport - best for many small/moderate files
    /// Advantages: multiplexing, lower latency, 0-RTT
    Quic,
    
    /// TCP transport - best for few large files
    /// Advantages: simpler, well-tested, better congestion control
    Tcp,
}

impl TransportProtocol {
    /// Returns the BLE service UUID for this protocol
    pub fn service_uuid(&self) -> &'static str {
        match self {
            TransportProtocol::Quic => QUIC_SERVICE_UUID,
            TransportProtocol::Tcp => TCP_SERVICE_UUID,
        }
    }
    
    /// Returns the BLE characteristic UUID for this protocol
    pub fn char_uuid(&self) -> &'static str {
        match self {
            TransportProtocol::Quic => QUIC_CHAR_UUID,
            TransportProtocol::Tcp => TCP_CHAR_UUID,
        }
    }
}

/* ========== Session Information ========== */

/// SessionTicket is advertised via BLE characteristic
/// Contains all info needed for receiver to connect via libp2p
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SessionTicket {
    /// The sender's libp2p peer ID
    pub peer_id: PeerId,
    
    /// List of multiaddresses where sender is listening
    pub addrs: Vec<Multiaddr>,
    
    /// Which transport protocol to use (QUIC or TCP)
    pub protocol: TransportProtocol,
    
    /// Random nonce for freshness/replay prevention
    pub nonce: u64,
    
    /// Ed25519 signature of (peer_id || addrs || protocol || nonce)
    #[serde(with = "BigArray")]
    pub sig: [u8; 64],
}

/* ========== File Transfer Metadata ========== */

/// Metadata for a single file being transferred
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileMetadata {
    /// Original filename
    pub name: String,
    
    /// File size in bytes
    pub size: u64,
    
    /// SHA256 hash of file contents (for verification)
    pub hash: Option<[u8; 32]>,
}

/// List of files to be transferred
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileList {
    /// Collection of file metadata
    pub files: Vec<FileMetadata>,
    
    /// Total size of all files combined
    pub total_size: u64,
    
    /// Actual file contents (name -> data)
    pub file_data: Vec<FileData>,
}

/// File content data
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileData {
    /// File index in the files list
    pub index: usize,
    
    /// File name
    pub name: String,
    
    /// File contents
    pub data: Vec<u8>,
}

/* ========== Transfer Protocol Messages ========== */

/// Request sent by receiver to initiate transfer
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TransferRequest {
    /// Request ID for correlation
    pub request_id: u64,
    
    /// Ready to receive?
    pub ready: bool,
}

/// Response sent by sender
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TransferResponse {
    /// Corresponding request ID
    pub request_id: u64,
    
    /// File list metadata
    pub file_list: FileList,
    
    /// Accepted or rejected
    pub accepted: bool,
}

/// Chunk of file data being transferred
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileChunk {
    /// Index of file in FileList
    pub file_index: usize,
    
    /// Chunk sequence number (0-based)
    pub chunk_number: u64,
    
    /// Total number of chunks for this file
    pub total_chunks: u64,
    
    /// Actual data bytes
    pub data: Vec<u8>,
}

/// Acknowledgment for received chunk
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ChunkAck {
    /// File index
    pub file_index: usize,
    
    /// Chunk that was received
    pub chunk_number: u64,
    
    /// Success or error
    pub success: bool,
}
