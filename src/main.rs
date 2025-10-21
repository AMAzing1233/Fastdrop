use btleplug::api::{Central, Manager as _, Peripheral, ScanFilter};
use btleplug::platform::Manager;
use libp2p::{Multiaddr, PeerId};
use serde::{Deserialize, Serialize};
use serde_cbor::from_slice;
use serde_json::to_string_pretty;
use std::{
    error::Error,
    io::{self, Write},
    time::Duration,
};
use tokio::time;
use uuid::Uuid;
use serde_big_array::BigArray;

/* ---------- Fast-drop UUIDs ---------- */
const SERVICE_UUID_STR: &str = "12345678-1234-5678-1234-56789ABCDEF0";
const CHAR_UUID_STR:    &str = "ABCDEFAB-CDEF-1234-5678-1234567890AB";

/* ---------- CBOR ticket layout with real types ---------- */
#[derive(Serialize, Deserialize, Debug)]
struct SessionTicket {
    peer_id: PeerId,           // who we are
    addrs:   Vec<Multiaddr>,   // how to reach us
    nonce:   u64,              // freshness
    #[serde(with = "BigArray")]
    sig:     [u8; 64],         // Ed25519 signature
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    /* 1. adapter ---------------------------------------------------------- */
    let manager  = Manager::new().await?;
    let adapter  = manager.adapters().await?
        .into_iter()
        .next()
        .ok_or("No Bluetooth adapters found")?;
    println!("Using adapter: {}", adapter.adapter_info().await?);

    /* 2. scan ------------------------------------------------------------- */
    adapter.start_scan(ScanFilter::default()).await?;
    println!("Scanning for 15 s …");
    time::sleep(Duration::from_secs(15)).await;
    adapter.stop_scan().await?;

    /* 3. filter for Fast-drop service + local-name ------------------------ */
    let target_service = Uuid::parse_str(SERVICE_UUID_STR)?;
    let mut fastdrop = Vec::new();
    for p in adapter.peripherals().await? {
        if let Some(props) = p.properties().await? {
            if props.services.contains(&target_service) && props.local_name.is_some() {
                fastdrop.push(p);
            }
        }
    }
    if fastdrop.is_empty() {
        println!("No Fast-drop peripherals found — exiting.");
        return Ok(());
    }

    println!("\nFound {} Fast-drop devices:", fastdrop.len());
    for (i, p) in fastdrop.iter().enumerate() {
        print_device_summary(i, p).await;
    }

    /* 4. user selection --------------------------------------------------- */
    print!("Select device numbers (comma/space): ");
    io::stdout().flush()?;
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    let picks: Vec<usize> = buf.split([',', ' ', '\n'])
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    if picks.is_empty() {
        println!("No valid selection — exiting.");
        return Ok(());
    }

    let char_uuid = Uuid::parse_str(CHAR_UUID_STR)?;

    /* 5. connect / read CBOR ticket -------------------------------------- */
    for idx in picks {
        if let Some(peripheral) = fastdrop.get(idx.wrapping_sub(1)) {
            println!("\n=== Device {idx} ===");
            if let Err(e) = handle_peripheral(peripheral, char_uuid).await {
                eprintln!("Error: {e}");
            }
        } else {
            eprintln!("Index {idx} out of range, skipping.");
        }
    }
    Ok(())
}

/* ---------- summary line helper ---------------------------------------- */
async fn print_device_summary(i: usize, p: &impl Peripheral) {
    let props = p.properties().await.unwrap_or(None);
    let addr  = p.address();
    let name  = props.as_ref()
        .and_then(|pr| pr.local_name.clone())
        .unwrap_or_else(|| "Unknown".into());
    println!("{:>2}. {}  {}", i + 1, addr, name);
    if let Some(pr) = props {
        if let Some(rssi) = pr.rssi {
            println!("      RSSI: {rssi} dBm");
        }
    }
}

/* ---------- connect, read, decode, disconnect -------------------------- */
async fn handle_peripheral(
    p: &impl Peripheral,
    char_uuid: Uuid,
) -> Result<(), Box<dyn Error>> {
    p.connect().await?;
    p.discover_services().await?;

    if let Some(ch) = p.characteristics().iter().find(|c| c.uuid == char_uuid) {
        let data = p.read(ch).await?;
        let ticket: SessionTicket = from_slice(&data)?;
        println!("🎁 Session-ticket (decoded):\n{}", to_string_pretty(&ticket)?);
    } else {
        println!("Characteristic {char_uuid} not found.");
    }

    p.disconnect().await?;
    Ok(())
}
