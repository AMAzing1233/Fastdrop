// libp2p networking layer for file transfer

use crate::protocol::{TransferRequest, TransferResponse, TransportProtocol};
use anyhow::{Context, Result};
use futures::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use libp2p::{
    identity::Keypair,
    noise,
    swarm::NetworkBehaviour,
    tcp, yamux, Multiaddr, Swarm, SwarmBuilder,
};
use libp2p_stream as stream;
use std::io;
use std::time::Duration;

/* ========== Stream Protocols ========== */

pub const TRANSFER_PROTOCOL: &str = "/fastdrop/transfer/1.0.0";

/* ========== Network Behaviour ========== */

#[derive(NetworkBehaviour)]
pub struct FileTransferBehaviour {
    pub stream: stream::Behaviour,
}

/* ========== Swarm Building ========== */

/// Build a swarm with QUIC transport
pub fn build_quic_swarm(keypair: Keypair) -> Result<Swarm<FileTransferBehaviour>> {
    let peer_id = keypair.public().to_peer_id();
    println!("🔑 Local PeerId: {}", peer_id);

    let swarm = SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_quic()
        .with_behaviour(|_key| {
            let behaviour = stream::Behaviour::new();
            FileTransferBehaviour { stream: behaviour }
        })
        .context("Failed to create behaviour")?
        .with_swarm_config(|cfg| {
            cfg.with_idle_connection_timeout(Duration::from_secs(300))
        })
        .build();

    Ok(swarm)
}

/// Build a swarm with TCP transport
pub fn build_tcp_swarm(keypair: Keypair) -> Result<Swarm<FileTransferBehaviour>> {
    let peer_id = keypair.public().to_peer_id();
    println!("🔑 Local PeerId: {}", peer_id);

    let swarm = SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )
        .context("Failed to configure TCP transport")?
        .with_behaviour(|_key| {
            let behaviour = stream::Behaviour::new();
            FileTransferBehaviour { stream: behaviour }
        })
        .context("Failed to create behaviour")?
        .with_swarm_config(|cfg| {
            cfg.with_idle_connection_timeout(Duration::from_secs(300))
        })
        .build();

    Ok(swarm)
}

/// Build a swarm based on protocol selection
pub fn build_swarm(
    keypair: Keypair,
    protocol: TransportProtocol,
) -> Result<Swarm<FileTransferBehaviour>> {
    match protocol {
        TransportProtocol::Quic => build_quic_swarm(keypair),
        TransportProtocol::Tcp => build_tcp_swarm(keypair),
    }
}

/* ========== Helper Functions ========== */

/// Start listening on appropriate addresses for the protocol
pub fn listen_on_protocol(
    swarm: &mut Swarm<FileTransferBehaviour>,
    protocol: TransportProtocol,
) -> Result<Vec<Multiaddr>> {
    let listen_addrs = Vec::new();

    match protocol {
        TransportProtocol::Quic => {
            // QUIC uses UDP
            let addr: Multiaddr = "/ip4/0.0.0.0/udp/0/quic-v1".parse()?;
            swarm.listen_on(addr)?;
        }
        TransportProtocol::Tcp => {
            // TCP
            let addr: Multiaddr = "/ip4/0.0.0.0/tcp/0".parse()?;
            swarm.listen_on(addr)?;
        }
    }

    Ok(listen_addrs)
}

/// Extract actual listening addresses from swarm
pub fn get_listen_addrs(swarm: &Swarm<FileTransferBehaviour>) -> Vec<Multiaddr> {
    swarm.listeners().cloned().collect()
}

/// Get stream control for opening/accepting streams
pub fn get_stream_control(swarm: &Swarm<FileTransferBehaviour>) -> stream::Control {
    swarm.behaviour().stream.new_control()
}

/// Write a request to a stream
pub async fn write_request<T>(stream: &mut T, request: TransferRequest) -> Result<()>
where
    T: AsyncWrite + Unpin,
{
    let data = serde_cbor::to_vec(&request)
        .context("Failed to serialize request")?;
    
    // Write length prefix
    let len = data.len() as u32;
    stream.write_all(&len.to_be_bytes()).await
        .context("Failed to write length")?;
    
    // Write data
    stream.write_all(&data).await
        .context("Failed to write request")?;
    
    stream.flush().await
        .context("Failed to flush stream")?;
    
    Ok(())
}

/// Read a request from a stream
pub async fn read_request<T>(stream: &mut T) -> Result<TransferRequest>
where
    T: AsyncRead + Unpin,
{
    // Read length prefix
    let mut len_bytes = [0u8; 4];
    stream.read_exact(&mut len_bytes).await
        .context("Failed to read length")?;
    let len = u32::from_be_bytes(len_bytes) as usize;
    
    // Read data
    let mut data = vec![0u8; len];
    stream.read_exact(&mut data).await
        .context("Failed to read request")?;
    
    serde_cbor::from_slice(&data)
        .context("Failed to deserialize request")
}

/// Write a response to a stream
pub async fn write_response<T>(stream: &mut T, response: TransferResponse) -> Result<()>
where
    T: AsyncWrite + Unpin,
{
    let data = serde_cbor::to_vec(&response)
        .context("Failed to serialize response")?;
    
    // Write length prefix
    let len = data.len() as u32;
    stream.write_all(&len.to_be_bytes()).await
        .context("Failed to write length")?;
    
    // Write data
    stream.write_all(&data).await
        .context("Failed to write response")?;
    
    stream.flush().await
        .context("Failed to flush stream")?;
    
    Ok(())
}

/// Read a response from a stream
pub async fn read_response<T>(stream: &mut T) -> Result<TransferResponse>
where
    T: AsyncRead + Unpin,
{
    // Read length prefix
    let mut len_bytes = [0u8; 4];
    stream.read_exact(&mut len_bytes).await
        .context("Failed to read length")?;
    let len = u32::from_be_bytes(len_bytes) as usize;
    
    // Read data
    let mut data = vec![0u8; len];
    stream.read_exact(&mut data).await
        .context("Failed to read response")?;
    
    serde_cbor::from_slice(&data)
        .context("Failed to deserialize response")
}

/* ========== Chunk Transfer via Stream ========== */

/// Send chunks over a raw stream
pub async fn send_chunks_over_stream<T>(
    stream: &mut T,
    chunks: Vec<crate::protocol::FileChunk>,
) -> Result<()>
where
    T: AsyncWrite + Unpin,
{
    for chunk in chunks {
        let data = serde_cbor::to_vec(&chunk)
            .context("Failed to serialize chunk")?;
        
        // Write length prefix
        let len = data.len() as u32;
        stream.write_all(&len.to_be_bytes()).await
            .context("Failed to write chunk length")?;
        
        // Write data
        stream.write_all(&data).await
            .context("Failed to write chunk")?;
    }
    stream.flush().await.context("Failed to flush stream")?;
    Ok(())
}

/// Receive chunks from a raw stream
pub async fn receive_chunks_from_stream<T>(
    stream: &mut T,
) -> Result<Vec<crate::protocol::FileChunk>>
where
    T: AsyncRead + Unpin,
{
    let mut chunks = Vec::new();
    
    loop {
        // Try to read length prefix
        let mut len_bytes = [0u8; 4];
        match stream.read_exact(&mut len_bytes).await {
            Ok(_) => {
                let len = u32::from_be_bytes(len_bytes) as usize;
                
                // Read data
                let mut data = vec![0u8; len];
                stream.read_exact(&mut data).await
                    .context("Failed to read chunk data")?;
                
                let chunk: crate::protocol::FileChunk = serde_cbor::from_slice(&data)
                    .context("Failed to deserialize chunk")?;
                chunks.push(chunk);
            }
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                break; // End of stream
            }
            Err(e) => return Err(e.into()),
        }
    }
    
    Ok(chunks)
}
