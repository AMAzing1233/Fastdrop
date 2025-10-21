// Receiver: Scans for BLE devices and receives files via libp2p

mod network;
mod protocol;
mod transfer;

use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::Manager;
use futures::StreamExt;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use libp2p::identity::Keypair;
use libp2p::request_response::{Event as RREvent, Message};
use libp2p::swarm::SwarmEvent;
use protocol::{SessionTicket, TransferRequest};
use serde_cbor::from_slice;
use std::{
    error::Error,
    io::{self, Write},
    path::PathBuf,
    time::Duration,
};
use tokio::time;
use uuid::Uuid;

/* ========== All UUIDs to scan for ========== */
const ALL_SERVICE_UUIDS: &[&str] = &[
    protocol::QUIC_SERVICE_UUID,
    protocol::TCP_SERVICE_UUID,
];

const ALL_CHAR_UUIDS: &[&str] = &[
    protocol::QUIC_CHAR_UUID,
    protocol::TCP_CHAR_UUID,
];

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!("🚀 Fastdrop Receiver");
    println!("====================\n");

    /* 1. Setup Bluetooth adapter */
    let manager = Manager::new().await?;
    let adapter = manager
        .adapters()
        .await?
        .into_iter()
        .next()
        .ok_or("No Bluetooth adapters found")?;
    println!("📡 Using adapter: {}", adapter.adapter_info().await?);

    /* 2. Scan for devices */
    adapter.start_scan(ScanFilter::default()).await?;
    println!("🔍 Scanning for 15 seconds...\n");
    time::sleep(Duration::from_secs(15)).await;
    adapter.stop_scan().await?;

    /* 3. Filter for Fastdrop devices (any of the 4 UUIDs) */
    let target_uuids: Vec<Uuid> = ALL_SERVICE_UUIDS
        .iter()
        .filter_map(|s| Uuid::parse_str(s).ok())
        .collect();

    println!("🔎 Filtering for Fastdrop devices...");
    println!("   Looking for UUIDs:");
    for uuid in &target_uuids {
        println!("      - {}", uuid);
    }
    println!();

    let mut fastdrop_devices = Vec::new();
    for p in adapter.peripherals().await? {
        if let Some(props) = p.properties().await? {
            // Debug: print all discovered devices
            let name = props.local_name.as_deref().unwrap_or("Unknown");
            let has_service = target_uuids
                .iter()
                .any(|uuid| props.services.contains(uuid));
            
            if has_service {
                println!("✓ Found Fastdrop device: {} ({})", name, p.address());
                fastdrop_devices.push(p);
            }
        }
    }

    if fastdrop_devices.is_empty() {
        println!("❌ No Fastdrop devices found");
        println!("   Make sure the sender is running and advertising");
        return Ok(());
    }

    println!("\n✅ Found {} Fastdrop device(s):\n", fastdrop_devices.len());
    for (i, p) in fastdrop_devices.iter().enumerate() {
        print_device_summary(i, p).await;
    }

    /* 4. User selection */
    print!("\n📱 Select device number (1-{}): ", fastdrop_devices.len());
    io::stdout().flush()?;
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    
    let selection: usize = buf
        .trim()
        .parse()
        .map_err(|_| "Invalid selection")?;

    if selection == 0 || selection > fastdrop_devices.len() {
        eprintln!("❌ Invalid device number");
        return Ok(());
    }

    let peripheral = &fastdrop_devices[selection - 1];
    println!("\n🔗 Connecting to device {}...", selection);

    /* 5. Connect and read session ticket */
    peripheral.connect().await?;
    peripheral.discover_services().await?;
    println!("✅ Connected\n");

    // Debug: List all discovered services and characteristics
    println!("🔍 Discovered services:");
    for service in peripheral.services() {
        println!("   Service: {}", service.uuid);
        for ch in peripheral.characteristics() {
            if ch.service_uuid == service.uuid {
                println!("      Char: {}", ch.uuid);
            }
        }
    }
    println!();

    // Try to find characteristic from any of the UUIDs
    let char_uuids: Vec<Uuid> = ALL_CHAR_UUIDS
        .iter()
        .filter_map(|s| Uuid::parse_str(s).ok())
        .collect();

    println!("🔎 Looking for Fastdrop characteristics:");
    for uuid in &char_uuids {
        println!("   - {}", uuid);
    }
    println!();

    let mut ticket_data = None;
    for uuid in &char_uuids {
        if let Some(ch) = peripheral.characteristics().iter().find(|c| c.uuid == *uuid) {
            println!("✓ Found matching characteristic: {}", uuid);
            ticket_data = Some(peripheral.read(ch).await?);
            println!("📥 Read {} bytes from characteristic", ticket_data.as_ref().unwrap().len());
            break;
        }
    }

    if ticket_data.is_none() {
        eprintln!("❌ No Fastdrop characteristic found among discovered characteristics");
        peripheral.disconnect().await?;
        return Ok(());
    }

    let ticket_data = ticket_data.unwrap();
    let ticket: SessionTicket = from_slice(&ticket_data)?;

    println!("🎫 Session Ticket:");
    println!("   Protocol: {:?}", ticket.protocol);
    println!("   Peer ID: {}", ticket.peer_id);
    println!("   Addresses: {}", ticket.addrs.len());
    for addr in &ticket.addrs {
        println!("      - {}", addr);
    }
    println!();

    peripheral.disconnect().await?;
    println!("🔌 Disconnected from BLE\n");

    /* 6. Setup libp2p with appropriate protocol */
    let keypair = Keypair::generate_ed25519();
    let mut swarm = network::build_swarm(keypair, ticket.protocol)?;

    println!("🌐 Building P2P connection...");

    /* 7. Dial the sender */
    for addr in &ticket.addrs {
        println!("📞 Dialing {}", addr);
        if let Err(e) = swarm.dial(addr.clone()) {
            eprintln!("   ⚠️  Failed: {}", e);
        }
    }

    /* 8. Wait for connection and request transfer */
    let mut connected_peer = None;

    println!("\n⏳ Waiting for P2P connection...\n");

    loop {
        match swarm.select_next_some().await {
            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                println!("✅ P2P connection established with {}", peer_id);
                connected_peer = Some(peer_id);

                // Send transfer request
                let request = TransferRequest {
                    request_id: 1,
                    ready: true,
                };

                println!("📨 Sending transfer request...");
                network::send_request(&mut swarm, peer_id, request);
            }
            SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                println!("❌ Connection closed with {}: {:?}", peer_id, cause);
                if Some(peer_id) == connected_peer {
                    break;
                }
            }
            SwarmEvent::Behaviour(network::FileTransferBehaviourEvent::RequestResponse(
                rr_event,
            )) => match rr_event {
                RREvent::Message { message, .. } => match message {
                    Message::Response { response, .. } => {
                        println!("📦 Received file list:");
                        println!("   Files: {}", response.file_list.files.len());
                        println!(
                            "   Total size: {}",
                            transfer::format_bytes(response.file_list.total_size)
                        );
                        println!();

                        // Write files to disk with progress bars
                        let multi_progress = MultiProgress::new();
                        let style = ProgressStyle::default_bar()
                            .template("{spinner:.green} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({percent}%) {msg}")
                            .unwrap()
                            .progress_chars("#>-");

                        for file_data in &response.file_list.file_data {
                            let pb = multi_progress.add(ProgressBar::new(file_data.data.len() as u64));
                            pb.set_style(style.clone());
                            pb.set_message(file_data.name.clone());
                            
                            let output_path = PathBuf::from(&file_data.name);
                            
                            // Create parent directories if needed
                            if let Some(parent) = output_path.parent() {
                                tokio::fs::create_dir_all(parent).await?;
                            }
                            
                            match tokio::fs::write(&output_path, &file_data.data).await {
                                Ok(_) => {
                                    pb.inc(file_data.data.len() as u64);
                                    pb.finish_with_message(format!("✓ {}", file_data.name));
                                }
                                Err(e) => {
                                    pb.finish_with_message(format!("✗ Failed: {}", e));
                                    eprintln!("      ❌ Failed to write {}: {}", 
                                        output_path.display(), 
                                        e
                                    );
                                }
                            }
                        }

                        println!("\n✅ Transfer complete!");
                        println!("   Received {} file(s)\n", response.file_list.file_data.len());
                        
                        // Exit after receiving files
                        break;
                    }
                    Message::Request { .. } => {
                        // Receiver doesn't expect requests
                    }
                },
                RREvent::OutboundFailure { error, .. } => {
                    eprintln!("❌ Request failed: {:?}", error);
                }
                RREvent::InboundFailure { error, .. } => {
                    eprintln!("❌ Inbound failure: {:?}", error);
                }
                RREvent::ResponseSent { .. } => {}
            },
            _ => {}
        }
    }

    println!("👋 Done!");
    Ok(())
}

/* ========== Helper Functions ========== */

async fn print_device_summary<P: btleplug::api::Peripheral>(i: usize, p: &P) {
    let props = p.properties().await.unwrap_or(None);
    let addr = p.address();
    let name = props
        .as_ref()
        .and_then(|pr| pr.local_name.clone())
        .unwrap_or_else(|| "Unknown".into());
    
    println!("{:>2}. {} - {}", i + 1, addr, name);
    
    if let Some(pr) = props {
        if let Some(rssi) = pr.rssi {
            println!("      RSSI: {} dBm", rssi);
        }
    }
}
