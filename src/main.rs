use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Manager, Peripheral};
use std::error::Error;
use std::time::Duration;
use tokio::time;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Initialize the Bluetooth manager
    let manager = Manager::new().await?;
    println!("Bluetooth manager initialized");

    // Get available adapters
    let adapters = manager.adapters().await?;
    if adapters.is_empty() {
        return Err("No Bluetooth adapters found".into());
    }

    // Use the first adapter
    let adapter = adapters.into_iter().next().unwrap();
    println!("Using adapter: {}", adapter.adapter_info().await?);

    // Start scanning with default filter (finds all device types)
    println!("Starting scan for all Bluetooth devices...");
    adapter.start_scan(ScanFilter::default()).await?;

    // Scan for 15 seconds
    let scan_duration = 15;
    println!("Scanning for {} seconds...", scan_duration);
    time::sleep(Duration::from_secs(scan_duration)).await;

    // Get discovered devices
    let peripherals = adapter.peripherals().await?;
    println!("\nFound {} devices:", peripherals.len());

    // Display information about each device
    for (i, peripheral) in peripherals.iter().enumerate() {
        let properties = peripheral.properties().await?;
        let address = peripheral.address();
        let name = properties
            .as_ref()
            .and_then(|p| p.local_name.clone())
            .unwrap_or_else(|| "Unknown".to_string());

        println!("Device {}: {} - {}", i+1, address, name);

        // Print detailed information if available
        if let Some(props) = &properties {
            if let Some(rssi) = props.rssi {
                println!("  RSSI: {} dBm", rssi);
            }

            if !props.manufacturer_data.is_empty() {
                println!("  Manufacturer data: {:?}", props.manufacturer_data);
            }

            if !props.services.is_empty() {
                println!("  Services: {:?}", props.services);
            }
        }
        println!("------------------------------");
    }

    // Stop scanning
    adapter.stop_scan().await?;
    println!("Scan complete");

    Ok(())
}