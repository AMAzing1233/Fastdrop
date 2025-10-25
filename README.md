## Hi, this is the Fastdrop Project, an AirDrop Alternative

How does it work?
Simple, it uses BLE to discover devices and then uses QUIC or TCP to transfer files (It uses a stream to be specific)

Does this need an internet connection?
Yes and no. While both devices need to be on the same network, that network does not need "internet"; it can work offline within the network, so it won't use your data

# How do I use it
Just clone this project
and run the following
``cargo run --bin sender <file name>``
``cargo run --bin receiver``

Then just select your device and IT WORKS!!!

This should work on all devices, be it Linux, Windows, MAC and any mobile phones

In my testing(On the same network), this app performs around 1.2 - 1.5 times faster due to the newer quic protocol
//Todo
- Make it more like aidrop (Ie fully offline support)

