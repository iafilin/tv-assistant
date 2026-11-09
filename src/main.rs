mod adb;
mod assets;
mod catalog;
mod ui;

use adb::Adb;
use std::env;
use std::net::Ipv4Addr;

fn main() {
    let arguments: Vec<String> = env::args().collect();
    match arguments.get(1).map(String::as_str) {
        Some("--scan") => match adb::discover_adb_devices() {
            Ok(devices) => {
                for device in devices {
                    println!("{device}");
                }
            }
            Err(error) => {
                eprintln!("{error:#}");
                std::process::exit(1);
            }
        },
        Some("--catalog") => {
            let Some(address) = arguments.get(2) else {
                eprintln!("Использование: tv-assistant --catalog IP");
                std::process::exit(2);
            };
            let ip: Ipv4Addr = address.parse().unwrap_or_else(|_| {
                eprintln!("Некорректный IPv4-адрес");
                std::process::exit(2);
            });
            let result = Adb::new()
                .and_then(|adb| adb.connect(ip, 5555))
                .and_then(|device| catalog::fetch_catalog(&device));
            match result {
                Ok(items) => {
                    for item in items {
                        println!(
                            "{} · {} · {} · installed={}",
                            item.name,
                            item.version,
                            catalog::format_size(item.size),
                            item.installed
                        );
                    }
                }
                Err(error) => {
                    eprintln!("{error:#}");
                    std::process::exit(1);
                }
            }
        }
        Some("--catalog-view") => {
            let Some(address) = arguments.get(2) else {
                eprintln!("Использование: tv-assistant --catalog-view IP");
                std::process::exit(2);
            };
            let ip: Ipv4Addr = address.parse().unwrap_or_else(|_| {
                eprintln!("Некорректный IPv4-адрес");
                std::process::exit(2);
            });
            match Adb::new().and_then(|adb| adb.connect(ip, 5555)) {
                Ok(device) => ui::run_catalog(Some(device)),
                Err(error) => {
                    eprintln!("{error:#}");
                    std::process::exit(1);
                }
            }
        }
        Some("--projectivy") => {
            let Some(address) = arguments.get(2) else {
                eprintln!("Использование: tv-assistant --projectivy IP");
                std::process::exit(2);
            };
            let ip: Ipv4Addr = address.parse().unwrap_or_else(|_| {
                eprintln!("Некорректный IPv4-адрес");
                std::process::exit(2);
            });
            let serial = format!("{ip}:5555");
            match Adb::new().and_then(|adb| adb.install_projectivy(&serial)) {
                Ok(message) => println!("{message}"),
                Err(error) => {
                    eprintln!("{error:#}");
                    std::process::exit(1);
                }
            }
        }
        Some("--disable") | Some("--restore") => {
            let restore = arguments[1] == "--restore";
            let (Some(address), Some(package)) = (arguments.get(2), arguments.get(3)) else {
                eprintln!(
                    "Использование: tv-assistant --disable IP PACKAGE | --restore IP PACKAGE"
                );
                std::process::exit(2);
            };
            let ip: Ipv4Addr = address.parse().unwrap_or_else(|_| {
                eprintln!("Некорректный IPv4-адрес");
                std::process::exit(2);
            });
            let serial = format!("{ip}:5555");
            let packages = vec![package.clone()];
            let result = Adb::new().and_then(|adb| {
                if restore {
                    adb.restore_packages(&serial, &packages)
                } else {
                    adb.disable_packages(&serial, &packages)
                }
            });
            match result {
                Ok(device) => {
                    let installed = device
                        .packages
                        .iter()
                        .find(|item| &item.name == package)
                        .map(|item| item.installed);
                    println!("{package}: installed={installed:?}");
                }
                Err(error) => {
                    eprintln!("{error:#}");
                    std::process::exit(1);
                }
            }
        }
        Some("--connect") => {
            let Some(address) = arguments.get(2) else {
                eprintln!("Использование: tv-assistant --connect IP [PORT]");
                std::process::exit(2);
            };
            let ip: Ipv4Addr = address.parse().unwrap_or_else(|_| {
                eprintln!("Некорректный IPv4-адрес");
                std::process::exit(2);
            });
            let port = arguments
                .get(3)
                .and_then(|value| value.parse().ok())
                .unwrap_or(5555);
            match Adb::new().and_then(|adb| adb.connect(ip, port)) {
                Ok(device) => ui::run(Some(device)),
                Err(error) => {
                    eprintln!("{error:#}");
                    std::process::exit(1);
                }
            }
        }
        Some("--inspect") => {
            let Some(address) = arguments.get(2) else {
                eprintln!("Использование: tv-assistant --inspect IP [PORT]");
                std::process::exit(2);
            };
            let ip: Ipv4Addr = address.parse().unwrap_or_else(|_| {
                eprintln!("Некорректный IPv4-адрес");
                std::process::exit(2);
            });
            let port = arguments
                .get(3)
                .and_then(|value| value.parse().ok())
                .unwrap_or(5555);
            match Adb::new().and_then(|adb| adb.connect(ip, port)) {
                Ok(device) => {
                    println!(
                        "{} {} · Android {} · {}",
                        device.brand, device.model, device.android, device.abi
                    );
                    println!("Серийный адрес: {}", device.serial);
                    println!("Лаунчер: {}", device.launcher);
                    println!("Пакетов: {}", device.packages.len());
                    for package in device.packages.iter().filter(|package| package.running) {
                        println!("Работает: {} ({})", package.label, package.name);
                    }
                }
                Err(error) => {
                    eprintln!("{error:#}");
                    std::process::exit(1);
                }
            }
        }
        _ => ui::run(None),
    }
}
