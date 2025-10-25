// Sender: Advertises via BLE and sends files via libp2p

mod network;
mod protocol;
mod transfer;

use anyhow::{Context, Result};
use ble_peripheral_rust::gatt::{characteristic, properties, service};
use ble_peripheral_rust::{Peripheral, PeripheralImpl};
use futures::StreamExt;
use libp2p::identity::Keypair;
use libp2p::swarm::SwarmEvent;
use libp2p::{PeerId, StreamProtocol};
use protocol::{SessionTicket, TransferResponse};
use std::collections::HashMap;
use std::env;
use std::path::PathBuf;
use tokio::signal;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<()> {
    println!("🚀 Fastdrop Sender");
    println!("==================\n");

    // 1. Get file paths from command line
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("Usage: sender <file1> [file2] [file3] ...");
        eprintln!("\nExample: sender document.pdf photo.jpg video.mp4");
        std::process::exit(1);
    }

    let file_paths: Vec<PathBuf> = args.iter().map(PathBuf::from).collect();
    println!("📁 Files to send: {}", file_paths.len());
    for path in &file_paths {
        println!("   - {}", path.display());
    }
    println!();

    // 2. Analyze files and determine protocol
    let (protocol, file_list) = transfer::analyze_files(&file_paths)
        .await
        .context("Failed to analyze files")?;

    println!(
        "📊 Total size: {} ({})\n",
        transfer::format_bytes(file_list.total_size),
        file_list.total_size
    );

    // Note: We don't load file contents into memory anymore
    // Files will be sent as chunks on-demand

    // 3. Setup libp2p swarm
    let keypair = Keypair::generate_ed25519();
    let peer_id = keypair.public().to_peer_id();
    
    let mut swarm = network::build_swarm(keypair.clone(), protocol)
        .context("Failed to build swarm")?;

    // 4. Start listening on appropriate transport
    let listen_addr = match protocol {
        protocol::TransportProtocol::Quic => "/ip4/0.0.0.0/udp/0/quic-v1".parse()?,
        protocol::TransportProtocol::Tcp => "/ip4/0.0.0.0/tcp/0".parse()?,
    };
    
    swarm.listen_on(listen_addr)
        .context("Failed to start listening")?;

    println!("⏳ Waiting for network to bind...\n");

    // Wait for NewListenAddr events to get actual bound addresses
    // We need to collect multiple addresses and filter out localhost
    let mut listen_addrs = Vec::new();
    let mut addr_count = 0;
    
    loop {
        tokio::select! {
            event = swarm.select_next_some() => {
                match event {
                    SwarmEvent::NewListenAddr { address, .. } => {
                        println!("🎧 Listening on: {}", address);
                        
                        // Filter out localhost addresses for the ticket
                        let addr_str = address.to_string();
                        if !addr_str.contains("127.0.0.1") && !addr_str.contains("::1") {
                            listen_addrs.push(address);
                            println!("   ✅ Added to ticket (non-localhost)");
                        } else {
                            println!("   ⚠️  Skipped (localhost)");
                        }
                        
                        addr_count += 1;
                        
                        // Wait for a short time to collect all addresses
                        // Usually we get 3-4 addresses (localhost, LAN, etc.)
                        if addr_count >= 3 {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            _ = tokio::time::sleep(Duration::from_secs(2)) => {
                // Timeout after 2 seconds even if we haven't gotten 3 addresses
                if !listen_addrs.is_empty() {
                    break;
                }
            }
        }
    }
    
    if listen_addrs.is_empty() {
        anyhow::bail!("No valid listen addresses obtained (all were localhost)");
    }

    println!();

    // 5. Create session ticket
    let nonce = rand::random::<u64>();
    
    // Create signature (simplified - in production should sign actual data)
    let mut sig = [0u8; 64];
    sig[..8].copy_from_slice(&nonce.to_le_bytes());
    
    let ticket = SessionTicket {
        peer_id,
        addrs: listen_addrs.clone(),
        protocol,
        nonce,
        sig,
    };

    // 6. Encode ticket as CBOR
    let ticket_cbor = serde_cbor::to_vec(&ticket)
        .context("Failed to encode session ticket")?;

    println!("🎫 Session ticket created ({} bytes)", ticket_cbor.len());
    println!("   Protocol: {:?}", protocol);
    println!("   PeerId: {}", peer_id);
    println!();

    // 7. Setup BLE advertising
    let (tx, _rx) = mpsc::channel::<_>(256);
    let mut peripheral = Peripheral::new(tx)
        .await
        .context("Failed to create BLE peripheral")?;

    println!("⏳ Waiting for Bluetooth adapter to power on...");
    while !peripheral.is_powered().await? {
        sleep(Duration::from_millis(100)).await;
    }
    println!("✅ Bluetooth adapter powered on\n");

    // 8. Setup GATT service with protocol-specific UUIDs
    let service_uuid = Uuid::parse_str(protocol.service_uuid())
        .context("Invalid service UUID")?;
    let char_uuid = Uuid::parse_str(protocol.char_uuid())
        .context("Invalid characteristic UUID")?;

    let characteristic = characteristic::Characteristic {
        uuid: char_uuid,
        properties: vec![properties::CharacteristicProperty::Read],
        permissions: vec![properties::AttributePermission::Readable],
        value: Some(ticket_cbor.into()),
        descriptors: vec![],
    };

    let gatt_service = service::Service {
        uuid: service_uuid,
        primary: true,
        characteristics: vec![characteristic],
    };

    peripheral
        .add_service(&gatt_service)
        .await
        .context("Failed to add GATT service")?;

    println!("📡 GATT service configured:");
    println!("   Service UUID: {}", service_uuid);
    println!("   Char UUID: {}", char_uuid);
    println!();

    // 9. Start advertising
    peripheral
        .start_advertising("Fastdrop", &[service_uuid])
        .await
        .context("Failed to start advertising")?;

    sleep(Duration::from_secs(1)).await;

    if !peripheral.is_advertising().await? {
        anyhow::bail!("Advertising failed to start");
    }

    println!("🔵 BLE advertising active!");
    println!("🔍 Receivers can now discover this device\n");
    println!("📦 Waiting for transfer requests...");
    println!("   (Press Ctrl+C to cancel)\n");

    // 10. Setup stream acceptor
    println!("🔍 Debug: Setting up stream acceptor...");
    let mut control = network::get_stream_control(&swarm);
    let protocol_stream = StreamProtocol::new(network::TRANSFER_PROTOCOL);
    println!("🔍 Debug: Protocol: {}", network::TRANSFER_PROTOCOL);
    
    let mut incoming = control.accept(protocol_stream)
        .context("Failed to accept incoming streams")?;
    
    println!("✅ Stream acceptor configured");

    // Clone data for the stream handler task
    let file_list_clone = file_list.clone();
    let file_paths_clone = file_paths.clone();
    
    // Spawn task to handle incoming streams
    println!("🔍 Debug: Spawning incoming stream handler...");
    tokio::spawn(async move {
        println!("🔍 Debug: Stream handler task started, waiting for incoming streams...");
        while let Some((peer, mut stream)) = incoming.next().await {
            println!("📨 Received stream from {}", peer);
            
            let file_list = file_list_clone.clone();
            let file_paths = file_paths_clone.clone();
            
            tokio::spawn(async move {
                println!("🔍 Debug: Spawned handler for stream from {}", peer);
                // Read request
                println!("🔍 Debug: Reading request from stream...");
                match network::read_request(&mut stream).await {
                    Ok(request) => {
                        println!("📨 Transfer request from {}", peer);
                        println!("   Request ID: {}", request.request_id);
                        
                        if request.ready {
                            println!("🔍 Debug: Creating transfer response...");
                            let response = TransferResponse {
                                request_id: request.request_id,
                                file_list: file_list.clone(),
                                accepted: true,
                            };
                            
                            // Send response with metadata
                            println!("🔍 Debug: Sending response with metadata...");
                            if let Err(e) = network::write_response(&mut stream, response).await {
                                eprintln!("❌ Failed to send response: {}", e);
                                return;
                            }
                            
                            println!("✅ Sent file list metadata to receiver");
                            println!("📤 Starting to send file chunks...");
                            
                            // Now send all files as chunks
                            for (file_index, path) in file_paths.iter().enumerate() {
                                println!("📄 Sending file {}/{}: {}", 
                                    file_index + 1, 
                                    file_paths.len(), 
                                    path.display()
                                );
                                
                                match transfer::send_file(path, file_index).await {
                                    Ok(chunks) => {
                                        println!("   📦 Sending {} chunks...", chunks.len());
                                        
                                        // Send each chunk
                                        if let Err(e) = network::send_chunks_over_stream(&mut stream, chunks).await {
                                            eprintln!("   ❌ Failed to send chunks: {}", e);
                                            return;
                                        }
                                        
                                        println!("   ✅ All chunks sent for file {}", file_index);
                                    }
                                    Err(e) => {
                                        eprintln!("   ❌ Failed to prepare file: {}", e);
                                        return;
                                    }
                                }
                            }
                            
                            println!("✅ All files sent successfully to {}", peer);
                        }
                    }
                    Err(e) => {
                        eprintln!("❌ Failed to read request: {}", e);
                    }
                }
            });
        }
    });

    // 11. Handle P2P connection events
    let mut pending_transfers: HashMap<PeerId, Vec<PathBuf>> = HashMap::new();

    println!("🔍 Debug: Entering main event loop...");
    loop {
        tokio::select! {
            event = swarm.select_next_some() => {
                println!("🔍 Debug: Received swarm event: {:?}", std::mem::discriminant(&event));
                match event {
                    SwarmEvent::NewListenAddr { address, .. } => {
                        println!("🎧 New listen address: {}", address);
                    }
                    SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                        println!("🤝 Connection established with {}", peer_id);
                        println!("   Endpoint: {:?}", endpoint);
                        pending_transfers.insert(peer_id, file_paths.clone());
                    }
                    SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                        println!("❌ Connection closed with {}: {:?}", peer_id, cause);
                        pending_transfers.remove(&peer_id);
                    }
                    SwarmEvent::IncomingConnection { send_back_addr, .. } => {
                        println!("📥 Incoming connection from: {:?}", send_back_addr);
                    }
                    SwarmEvent::IncomingConnectionError { send_back_addr, error, .. } => {
                        eprintln!("❌ Incoming connection error from {:?}: {}", send_back_addr, error);
                    }
                    SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                        eprintln!("❌ Outgoing connection error to {:?}: {}", peer_id, error);
                    }
                    event => {
                        println!("🔍 Debug: Other event: {:?}", event);
                    }
                }
            }
            _ = signal::ctrl_c() => {
                println!("\n\n🛑 Received Ctrl+C, shutting down...");
                break;
            }
        }
    }

    println!("👋 Goodbye!");
    Ok(())
}
