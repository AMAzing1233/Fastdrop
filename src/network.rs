// libp2p networking layer for file transfer

use crate::protocol::{TransferRequest, TransferResponse, TransportProtocol};
use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use libp2p::{
    identity::Keypair,
    noise, request_response,
    swarm::NetworkBehaviour,
    tcp, yamux, Multiaddr, PeerId, StreamProtocol, Swarm, SwarmBuilder,
};
use std::io;
use std::time::Duration;

/* ========== Stream Protocols ========== */

pub const TRANSFER_PROTOCOL: &str = "/fastdrop/transfer/1.0.0";

/* ========== Custom Codec for Request-Response ========== */

#[derive(Clone)]
pub struct TransferCodec;

#[async_trait]
impl request_response::Codec for TransferCodec {
    type Protocol = StreamProtocol;
    type Request = TransferRequest;
    type Response = TransferResponse;

    async fn read_request<T>(
        &mut self,
        _protocol: &Self::Protocol,
        io: &mut T,
    ) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        // Read length prefix (4 bytes)
        let mut len_bytes = [0u8; 4];
        io.read_exact(&mut len_bytes).await?;
        let len = u32::from_be_bytes(len_bytes) as usize;
        
        // Read data
        let mut data = vec![0u8; len];
        io.read_exact(&mut data).await?;
        
        serde_cbor::from_slice(&data)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    async fn read_response<T>(
        &mut self,
        _protocol: &Self::Protocol,
        io: &mut T,
    ) -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send,
    {
        // Read length prefix (4 bytes)
        let mut len_bytes = [0u8; 4];
        io.read_exact(&mut len_bytes).await?;
        let len = u32::from_be_bytes(len_bytes) as usize;
        
        // Read data
        let mut data = vec![0u8; len];
        io.read_exact(&mut data).await?;
        
        serde_cbor::from_slice(&data)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    async fn write_request<T>(
        &mut self,
        _protocol: &Self::Protocol,
        io: &mut T,
        req: Self::Request,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        let data = serde_cbor::to_vec(&req)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        
        // Write length prefix
        let len = data.len() as u32;
        io.write_all(&len.to_be_bytes()).await?;
        
        // Write data
        io.write_all(&data).await?;
        io.flush().await
    }

    async fn write_response<T>(
        &mut self,
        _protocol: &Self::Protocol,
        io: &mut T,
        res: Self::Response,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        let data = serde_cbor::to_vec(&res)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        
        // Write length prefix
        let len = data.len() as u32;
        io.write_all(&len.to_be_bytes()).await?;
        
        // Write data
        io.write_all(&data).await?;
        io.flush().await
    }
}

/* ========== Network Behaviour ========== */

#[derive(NetworkBehaviour)]
pub struct FileTransferBehaviour {
    pub request_response: request_response::Behaviour<TransferCodec>,
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
            let protocols = [(
                StreamProtocol::new(TRANSFER_PROTOCOL),
                request_response::ProtocolSupport::Full,
            )];
            
            let cfg = request_response::Config::default()
                .with_request_timeout(Duration::from_secs(300)); // 5 min timeout

            let behaviour = request_response::Behaviour::with_codec(
                TransferCodec,
                protocols,
                cfg,
            );

            FileTransferBehaviour {
                request_response: behaviour,
            }
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
            let protocols = [(
                StreamProtocol::new(TRANSFER_PROTOCOL),
                request_response::ProtocolSupport::Full,
            )];
            
            let cfg = request_response::Config::default()
                .with_request_timeout(Duration::from_secs(300)); // 5 min timeout

            let behaviour = request_response::Behaviour::with_codec(
                TransferCodec,
                protocols,
                cfg,
            );

            FileTransferBehaviour {
                request_response: behaviour,
            }
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

/// Send a transfer request to a peer
pub fn send_request(
    swarm: &mut Swarm<FileTransferBehaviour>,
    peer: PeerId,
    request: TransferRequest,
) -> request_response::OutboundRequestId {
    swarm
        .behaviour_mut()
        .request_response
        .send_request(&peer, request)
}

/// Send a transfer response
pub fn send_response(
    swarm: &mut Swarm<FileTransferBehaviour>,
    channel: request_response::ResponseChannel<TransferResponse>,
    response: TransferResponse,
) -> Result<()> {
    swarm
        .behaviour_mut()
        .request_response
        .send_response(channel, response)
        .map_err(|_| anyhow::anyhow!("Failed to send response"))
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
