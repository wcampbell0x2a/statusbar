use std::fmt::Write;
use std::net::IpAddr;
use std::path::Path;
use std::process::Command;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Local};
use clap::Parser;
use local_ip_address::list_afinet_netifas;
use sysinfo::{System, Users, get_current_pid};

const BAT0_PATH: &str = "/sys/class/power_supply/BAT0/capacity";
const BAT1_PATH: &str = "/sys/class/power_supply/BAT1/capacity";
const BAT0_POWER_PATH: &str = "/sys/class/power_supply/BAT0/power_now";
const BAT1_POWER_PATH: &str = "/sys/class/power_supply/BAT1/power_now";
const AC_ONLINE_PATH: &str = "/sys/class/power_supply/AC/online";

/// Get WiFi SSID for a network interface
/// Returns Some(ssid) if the interface is WiFi and connected, None otherwise
fn get_wifi_ssid(interface: &str) -> Option<String> {
    use nl80211::Socket;

    Socket::connect()
        .ok()
        .and_then(|mut socket| socket.get_interfaces_info().ok())
        .and_then(|interfaces| {
            interfaces.into_iter().find(|iface| {
                let name = iface
                    .name
                    .as_ref()
                    .and_then(|name| String::from_utf8(name.clone()).ok())
                    .map(|s| s.trim_end_matches('\0').to_string());

                name.map(|name| name == interface).unwrap_or(false)
            })
        })
        .and_then(|iface| {
            // Extract SSID if available
            iface.ssid.and_then(|ssid| String::from_utf8(ssid).ok())
        })
}

/// Read battery capacity from a given path
fn read_battery_capacity(path: &str) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
}

/// Read power consumption from a given path (in microwatts)
fn read_power_uw(path: &str) -> Option<u64> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
}

/// Calculate memory usage percentage from /proc/meminfo
fn get_memory_usage_percent() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;

    let mut mem_total = None;
    let mut mem_available = None;

    for line in meminfo.lines() {
        let parts: Vec<&str> = line.split_ascii_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }

        match parts[0] {
            "MemTotal:" => mem_total = parts[1].parse::<u64>().ok(),
            "MemAvailable:" => mem_available = parts[1].parse::<u64>().ok(),
            _ => {}
        }

        if mem_total.is_some() && mem_available.is_some() {
            break;
        }
    }

    let total = mem_total?;
    let available = mem_available?;

    Some(((total - available) * 100) / total)
}

/// Get AC adapter online status
fn get_ac_online_status(path: &str) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse::<u8>().ok())
        .map(|status| status == 1)
        .unwrap_or(false)
}

/// Format IP addresses with WiFi SSID if available
fn format_ip_addresses(interfaces: &[String]) -> String {
    let network_interfaces = list_afinet_netifas().unwrap_or_default();
    let mut ip_addresses: Vec<(String, String)> = vec![];

    for (name, ip) in network_interfaces
        .iter()
        .filter(|(name, ip)| interfaces.contains(name) && matches!(ip, IpAddr::V4(_)))
    {
        let ip_str = ip.to_string();
        if !ip_addresses
            .iter()
            .any(|(_, existing_ip)| *existing_ip == ip_str)
        {
            ip_addresses.push((name.clone(), ip_str));
        }
    }

    let mut result = String::from("[");
    for (i, (interface, address)) in ip_addresses.iter().enumerate() {
        result.push_str(address);

        if let Some(ssid) = get_wifi_ssid(interface) {
            result.push_str(&format!("[{}]", ssid));
        }

        if i != ip_addresses.len() - 1 {
            result.push_str(", ");
        }
    }
    result.push(']');
    result
}

/// Format battery status string
fn format_battery_status(bat0: &str, bat1: &str) -> String {
    let mut battery_levels = Vec::new();

    if !bat0.is_empty() {
        battery_levels.push(format!("{}%", bat0));
    }
    if !bat1.is_empty() {
        battery_levels.push(format!("{}%", bat1));
    }

    if battery_levels.is_empty() {
        String::new()
    } else {
        format!(" bat [{}],", battery_levels.join(", "))
    }
}

#[derive(Debug, Parser)]
#[command(version)]
struct Cli {
    /// network interface for display of ip addresses
    #[arg(long)]
    interface: Vec<String>,

    /// override return from first user in sys.users()
    #[arg(long)]
    username: Option<String>,
}

fn main() {
    let args = Cli::parse();

    // test optional features
    let battery_00_enable = Path::new(BAT0_PATH).exists();
    let battery_01_enable = Path::new(BAT1_PATH).exists();
    let bat0_power_enable = Path::new(BAT0_POWER_PATH).exists();
    let bat1_power_enable = Path::new(BAT1_POWER_PATH).exists();
    let ac_online_enable = Path::new(AC_ONLINE_PATH).exists();

    // start
    let (ip_addresses_tx, ip_addresses_rx) = channel();
    let (bat0_tx, bat0_rx) = channel();
    let (bat1_tx, bat1_rx) = channel();
    let (wattage_tx, wattage_rx) = channel();
    let (ac_online_tx, ac_online_rx) = channel();
    let (mem_tx, mem_rx) = channel();
    let (cpu_tx, cpu_rx) = channel();
    let m_sys = Arc::new(Mutex::new(System::new_all()));

    // First call to sys functions, grabbing host_name and user name, and also ip addresses
    let (sys_host_name, sys_user_name) = {
        let mut sys = m_sys.lock().unwrap_or_else(|e| e.into_inner());

        sys.refresh_all();

        let pid = loop {
            match get_current_pid() {
                Ok(pid) => break pid,
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
            }
        };

        // overide sys.users()
        let name = if let Some(username) = &args.username {
            username.clone()
        } else if let Some(process) = sys.process(pid) {
            if let Some(user_id) = process.user_id() {
                let users = Users::new_with_refreshed_list();
                if let Some(user) = users.iter().find(|u| u.id() == user_id) {
                    user.name().to_string()
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        let hostname = loop {
            match System::host_name() {
                Some(name) => break name,
                None => {
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
            }
        };

        (hostname, name)
    };

    // Thread updating every n seconds
    std::thread::scope(|x| {
        x.spawn(move || {
            loop {
                // Battery 0
                if battery_00_enable && let Some(bat0) = read_battery_capacity(BAT0_PATH) {
                    while bat0_tx.send(bat0.clone()).is_err() {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }

                // Battery 1
                if battery_01_enable && let Some(bat1) = read_battery_capacity(BAT1_PATH) {
                    while bat1_tx.send(bat1.clone()).is_err() {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }

                // Total Wattage
                let mut total_wattage_uw: u64 = 0;
                if bat0_power_enable && let Some(power_uw) = read_power_uw(BAT0_POWER_PATH) {
                    total_wattage_uw += power_uw;
                }
                if bat1_power_enable && let Some(power_uw) = read_power_uw(BAT1_POWER_PATH) {
                    total_wattage_uw += power_uw;
                }
                // Convert from microwatts to watts
                let total_wattage = total_wattage_uw as f64 / 1_000_000.0;
                while wattage_tx.send(total_wattage).is_err() {
                    std::thread::sleep(Duration::from_millis(100));
                }

                // AC Online status
                let ac_online = if ac_online_enable {
                    get_ac_online_status(AC_ONLINE_PATH)
                } else {
                    false
                };
                while ac_online_tx.send(ac_online).is_err() {
                    std::thread::sleep(Duration::from_millis(100));
                }

                // Ram usage
                if let Some(memory_usage_percent) = get_memory_usage_percent() {
                    while mem_tx.send(memory_usage_percent).is_err() {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }

                // Cpu Usage
                let mut sys = m_sys.lock().unwrap_or_else(|e| e.into_inner());
                sys.refresh_cpu_all();
                let new_avg_cpu_usage: f32 =
                    ((sys.cpus().iter().map(|cpu| cpu.cpu_usage()).sum::<f32>())
                        / sys.cpus().len() as f32)
                        .ceil();
                while cpu_tx.send(new_avg_cpu_usage).is_err() {
                    std::thread::sleep(Duration::from_millis(100));
                }
                drop(sys);

                std::thread::sleep(Duration::from_secs(1));

                // IP Address
                let ip_addresses_string = format_ip_addresses(&args.interface);
                while ip_addresses_tx.send(ip_addresses_string.clone()).is_err() {
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        });

        // X updater thread
        x.spawn(move || {

            // Status string
            let mut last_bat0 = String::new();
            let mut last_bat1 = String::new();
            let mut last_wattage = 0.0;
            let mut last_ac_online = false;
            let mut last_mem_usage = 0;
            let mut last_cpu_usage = 0.0;
            let mut last_addrs = String::new();

            let mut status = String::new();

            loop {
                status.clear();
                // Get the time and make the status message
                let local: DateTime<Local> = Local::now();

                // Battery
                if let Ok(bat0) = bat0_rx.try_recv()
                    && battery_00_enable
                {
                    last_bat0 = bat0;
                }
                if let Ok(bat1) = bat1_rx.try_recv()
                    && battery_01_enable
                {
                    last_bat1 = bat1;
                }
                let battery_s = format_battery_status(&last_bat0, &last_bat1);

                // Wattage
                if let Ok(wattage) = wattage_rx.try_recv() {
                    last_wattage = wattage;
                }

                // AC Online
                if let Ok(ac_online) = ac_online_rx.try_recv() {
                    last_ac_online = ac_online;
                }

                // Mem
                if let Ok(mem_usage) = mem_rx.try_recv() {
                    last_mem_usage = mem_usage;
                }

                // Cpu
                if let Ok(cpu_usage) = cpu_rx.try_recv() {
                    last_cpu_usage = cpu_usage;
                }

                // Ip
                if let Ok(ip_addrs) = ip_addresses_rx.try_recv() {
                    last_addrs = ip_addrs;
                }

                let ac_indicator = if last_ac_online { " [AC]" } else { "" };
                if write!(
                    status,
                    "[{sys_host_name}][{sys_user_name}] => cpu {last_cpu_usage:02}%, mem {last_mem_usage:02}%, net {last_addrs},{battery_s} pwr {last_wattage:.1}W{ac_indicator}, {}",
                    local.format("%F %T")
                )
                .is_err()
                {
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }

                loop {
                    match Command::new("xsetroot")
                        .args(["-name", &status])
                        .status()
                    {
                        Ok(_) => break,
                        Err(_) => {
                            std::thread::sleep(Duration::from_millis(100));
                            continue;
                        }
                    }
                }

                std::thread::sleep(Duration::from_secs(1));
            }
        });
    });
}
