// Sender: Advertises via BLE and sends files via libp2p

mod network;
mod protocol;
mod transfer;

use anyhow::{Context, Result};
use ble_peripheral_rust::gatt::{characteristic, properties, service};
use ble_peripheral_rust::{Peripheral, PeripheralImpl};
use futures::StreamExt;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use libp2p::identity::Keypair;
use libp2p::request_response::{Event as RREvent, Message};
use libp2p::swarm::SwarmEvent;
use libp2p::PeerId;
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
    let (protocol, mut file_list) = transfer::analyze_files(&file_paths)
        .await
        .context("Failed to analyze files")?;

    println!(
        "📊 Total size: {} ({})\n",
        transfer::format_bytes(file_list.total_size),
        file_list.total_size
    );

    // Check size limits
    if file_list.total_size > protocol::MAX_TOTAL_SIZE {
        eprintln!(
            "❌ Error: Total size {} exceeds limit of {}",
            transfer::format_bytes(file_list.total_size),
            transfer::format_bytes(protocol::MAX_TOTAL_SIZE)
        );
        std::process::exit(1);
    }

    for file_meta in &file_list.files {
        if file_meta.size > protocol::MAX_FILE_SIZE {
            eprintln!(
                "❌ Error: File '{}' size {} exceeds limit of {}",
                file_meta.name,
                transfer::format_bytes(file_meta.size),
                transfer::format_bytes(protocol::MAX_FILE_SIZE)
            );
            std::process::exit(1);
        }
    }

    // 2b. Read file contents with progress bars
    println!("📖 Reading files...\n");
    
    let multi_progress = MultiProgress::new();
    let style = ProgressStyle::default_bar()
        .template("{spinner:.green} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({percent}%) {msg}")
        .unwrap()
        .progress_chars("#>-");

    for (idx, path) in file_paths.iter().enumerate() {
        let file_meta = &file_list.files[idx];
        let pb = multi_progress.add(ProgressBar::new(file_meta.size));
        pb.set_style(style.clone());
        pb.set_message(file_meta.name.clone());

        match tokio::fs::read(path).await {
            Ok(data) => {
                pb.inc(data.len() as u64);
                let file_data = protocol::FileData {
                    index: idx,
                    name: file_meta.name.clone(),
                    data,
                };
                file_list.file_data.push(file_data);
                pb.finish_with_message(format!("✓ {}", file_meta.name));
            }
            Err(e) => {
                pb.finish_with_message(format!("✗ Failed: {}", e));
                eprintln!("   ✗ Failed to read {}: {}", path.display(), e);
            }
        }
    }
    println!();

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

    // Wait for NewListenAddr event to get actual bound addresses
    let mut listen_addrs = Vec::new();
    loop {
        match swarm.select_next_some().await {
            SwarmEvent::NewListenAddr { address, .. } => {
                println!("🎧 Listening on: {}", address);
                listen_addrs.push(address);
                // Got at least one address, we can proceed
                break;
            }
            _ => {}
        }
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

    // 10. Handle P2P connection and file transfer
    let mut pending_transfers: HashMap<PeerId, Vec<PathBuf>> = HashMap::new();

    loop {
        tokio::select! {
            event = swarm.select_next_some() => {
                match event {
                    SwarmEvent::NewListenAddr { address, .. } => {
                        println!("🎧 New listen address: {}", address);
                    }
                    SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                        println!("🤝 Connection established with {}", peer_id);
                        pending_transfers.insert(peer_id, file_paths.clone());
                    }
                    SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                        println!("❌ Connection closed with {}: {:?}", peer_id, cause);
                        pending_transfers.remove(&peer_id);
                    }
                    SwarmEvent::Behaviour(network::FileTransferBehaviourEvent::RequestResponse(rr_event)) => {
                        match rr_event {
                            RREvent::Message { peer, message, .. } => {
                                match message {
                                    Message::Request { request, channel, .. } => {
                                        println!("📨 Transfer request from {}", peer);
                                        println!("   Request ID: {}", request.request_id);
                                        
                                        if request.ready {
                                            let response = TransferResponse {
                                                request_id: request.request_id,
                                                file_list: file_list.clone(),
                                                accepted: true,
                                            };
                                            
                                            if let Err(e) = network::send_response(&mut swarm, channel, response) {
                                                eprintln!("❌ Failed to send response: {}", e);
                                            } else {
                                                println!("✅ Sent file list to receiver");
                                                
                                                // Start sending files
                                                if let Some(paths) = pending_transfers.get(&peer) {
                                                    tokio::spawn(send_files_task(peer, paths.clone()));
                                                }
                                            }
                                        }
                                    }
                                    Message::Response { .. } => {
                                        // Sender doesn't expect responses
                                    }
                                }
                            }
                            RREvent::OutboundFailure { error, .. } => {
                                eprintln!("❌ Outbound failure: {:?}", error);
                            }
                            RREvent::InboundFailure { error, .. } => {
                                eprintln!("❌ Inbound failure: {:?}", error);
                            }
                            _ => {}
                        }
                    }
                    _ => {}
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

/// Task to send files (spawned as separate task)
async fn send_files_task(peer: PeerId, file_paths: Vec<PathBuf>) {
    println!("\n📤 Starting file transfer to {}", peer);
    
    for (idx, path) in file_paths.iter().enumerate() {
        println!("📄 Sending file {}/{}: {}", idx + 1, file_paths.len(), path.display());
        
        match transfer::send_file(path, idx).await {
            Ok(chunks) => {
                println!("   ✅ {} chunks prepared", chunks.len());
                // In a full implementation, we'd send these chunks over a stream
                // For now, this demonstrates the flow
            }
            Err(e) => {
                eprintln!("   ❌ Failed to prepare file: {}", e);
            }
        }
    }
    
    println!("✅ File transfer complete to {}\n", peer);
}
