# Fastdrop Usage Guide

## Quick Start

### Sender (Device with files to send)

1. **Start the sender with files:**
   ```powershell
   cargo run --bin sender -- file1.txt file2.pdf image.jpg
   ```

2. **Wait for setup:**
   - Network will bind to QUIC or TCP
   - Bluetooth adapter will power on
   - BLE advertising will start
   - You'll see: `🔵 BLE advertising active!`

3. **Keep it running** until receiver connects

### Receiver (Device receiving files)

1. **Run the receiver:**
   ```powershell
   cargo run --bin receiver
   ```

2. **Scanning process:**
   - Scans for 15 seconds
   - Filters for Fastdrop devices only
   - Shows list of available Fastdrop senders

3. **Select device:**
   - Enter the device number (e.g., `1`)
   - Press Enter

4. **File transfer:**
   - Receiver connects via BLE
   - Reads session ticket (P2P connection info)
   - Establishes P2P connection (QUIC or TCP)
   - Receives file list
   - Downloads files

## Protocol Selection (Automatic)

The sender automatically chooses:
- **QUIC**: Many files (>5) OR small total size (<100MB)
  - Better for: Multiple small files, lower latency
- **TCP**: Few files (≤5) AND large total size (≥100MB)
  - Better for: Large single files, reliable transfer

## Troubleshooting

### "No Fastdrop devices found"
- Make sure sender is running and advertising
- Check that Bluetooth is enabled on both devices
- Ensure devices are within Bluetooth range

### "No Fastdrop characteristic found"
- The BLE connection succeeded but couldn't read session data
- Check that sender completed full startup (see `🔵 BLE advertising active!`)

### Network binding issues
- Sender needs UDP port for QUIC or TCP port for TCP
- Check firewall settings if connection fails

## Example Output

### Sender:
```
🚀 Fastdrop Sender
📁 Files to send: 1
   - test.txt
📊 Analysis: 1 files, 34 bytes total → Using Quic
🔑 Local PeerId: 12D3KooW...
🎧 Listening on: /ip4/192.168.9.170/udp/61959/quic-v1
🔵 BLE advertising active!
```

### Receiver:
```
🚀 Fastdrop Receiver
🔍 Scanning for 15 seconds...
✓ Found Fastdrop device: Fastdrop (AA:BB:CC:DD:EE:FF)
✅ Found 1 Fastdrop device(s):
 1. AA:BB:CC:DD:EE:FF - Fastdrop
📱 Select device number (1-1): 1
🔗 Connecting to device 1...
✅ Connected
📥 Read 173 bytes from characteristic
🎫 Session Ticket:
   Protocol: Quic
   Peer ID: 12D3KooW...
```
