use std::error::Error;
use tokio::sync::mpsc::channel;
use uuid::Uuid;
use tokio::{signal, time};
use std::time::Duration;
use ble_peripheral_rust::{Peripheral, PeripheralImpl};
use ble_peripheral_rust::gatt::{
    service,
    characteristic,
    properties,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // 1) Set up the event channel and peripheral
    let (tx, _rx) = channel::<_>(256);
    let mut peripheral = Peripheral::new(tx).await?;
    println!("Waiting for adapter to power on…");
    while !peripheral.is_powered().await? {}
    println!("Adapter is powered on.");

    // 2) Define a custom 128-bit service UUID and a single characteristic UUID
    let service_uuid = Uuid::parse_str("12345678-1234-5678-1234-56789ABCDEF0")?;
    let char_uuid    = Uuid::parse_str("ABCDEFAB-CDEF-1234-5678-1234567890AB")?;

    // 3) Build a service with one read-only characteristic
    let characteristic = characteristic::Characteristic {
        uuid: char_uuid,
        properties: vec![properties::CharacteristicProperty::Read],
        permissions: vec![properties::AttributePermission::Readable],
        value: Some(b"OK".to_vec().into()),
        descriptors: vec![],
    };
    let service = service::Service {
        uuid: service_uuid,
        primary: true,
        characteristics: vec![characteristic],
    };

    // 4) Register the service with the OS
    peripheral.add_service(&service).await?;
    println!("GATT service {service_uuid} added.");

    // 5) Now start advertising with that service UUID
    peripheral
        .start_advertising("Fastdrop", &[service_uuid])
        .await?;
    println!("Called start_advertising…");

    // small pause to let the stack settle
    time::sleep(Duration::from_secs(2)).await;

    // 6) Verify if advertising really kicked off
    if peripheral.is_advertising().await? {
        println!("✅ Advertising as “Fastdrop” is now active!");
    } else {
        eprintln!("❌ Advertising still false—check for errors above.");
        return Err("Failed to start advertising".into());
    }

    // 7) Keep the program alive (and advertising) until you hit Ctrl+C
    println!("🔴 Advertising indefinitely. Press Ctrl+C to stop.");
    signal::ctrl_c().await?;
    println!("\n🛑 Received Ctrl+C, shutting down.");

    Ok(())
}
