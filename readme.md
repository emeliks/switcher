# Switcher

Switcher is a Rust based home automation system. Written in Rust (and a little js for frontend). Can be run on variety of operating systems such as Windows, Macos, Linux.

## Why Rust?

Rust is a great programming language, allowing to create fast, solid and reliable systems working in near c speed. Creating code in Rust is sometimes painful, but also a great adventure.

## Features

- contains implementation of protocols useed in communication witch smart devices, such as: Mqtt (3.1 & 5.0) client and server, Http client and server, Zigbee (over EZSP) concentrator,
- asynchronous (based on AsyncStd), multithreaded,
- built in expression parser allowing creating own automations,
- definition of connected devices is placed in system config file. No device dependent code is hardcoded,
- can be run on Linux or Windows machine or on Android based device such as NS Panel Pro,
- independent from 3rd party software such as Mosquitto, Zigbe To Mqtt, etc.,

## Build

### Linux

```sh
cd switcher
cargo build -r
```

### Android ARM

```sh
cd switcher
cross build --target armv7-linux-androideabi --release
```

## Config

Sample config file is placed in directory switcher. Config contains json data with added // comments.

## Run

```sh
cd switcher
./target/release/switcher ./config.json
```

