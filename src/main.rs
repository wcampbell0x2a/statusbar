#![feature(string_remove_matches)]
#![feature(let_chains)]

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
use sysinfo::{get_current_pid, ProcessExt, System, SystemExt, UserExt};

const BAT0_PATH: &str = "/sys/class/power_supply/BAT0/capacity";
const BAT1_PATH: &str = "/sys/class/power_supply/BAT1/capacity";

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

    // start
    let (ip_addresses_tx, ip_addresses_rx) = channel();
    let (bat0_tx, bat0_rx) = channel();
    let (bat1_tx, bat1_rx) = channel();
    let (mem_tx, mem_rx) = channel();
    let (cpu_tx, cpu_rx) = channel();
    let m_sys = Arc::new(Mutex::new(System::new_all()));

    // First call to sys functions, grabbing host_name and user name, and also ip addresses
    let (sys_host_name, sys_user_name) = {
        let mut sys = m_sys.lock().unwrap();

        sys.refresh_all();

        let pid = get_current_pid().unwrap();

        // overide sys.users()
        let mut name = "";
        if let Some(username) = &args.username {
            name = username;
        } else {
            if let Some(process) = sys.process(pid) {
                if let Some(user_id) = process.user_id() {
                    name = sys.get_user_by_id(user_id).unwrap().name();
                }
            }
        }

        (sys.host_name().unwrap(), name.to_string())
    };

    // Thread updating every n seconds
    std::thread::scope(|x| {
        x.spawn(move || {
            loop {
                // Battery 0
                if battery_00_enable {
                    let mut bat0 = std::fs::read_to_string(BAT0_PATH).unwrap();
                    bat0.remove_matches('\n');
                    bat0_tx.send(bat0).unwrap();
                }

                // Battery 1
                if battery_01_enable {
                    let mut bat1 = std::fs::read_to_string(BAT1_PATH).unwrap();
                    bat1.remove_matches('\n');
                    bat1_tx.send(bat1).unwrap();
                }

                // Ram usage
                let ram = std::fs::read_to_string("/proc/meminfo").unwrap();
                let lines = &ram.split('\n').collect::<Vec<&str>>();
                // Memory Total
                let mem_total = lines[0].split_ascii_whitespace().collect::<Vec<&str>>();
                let mem_total = mem_total[1].parse::<u64>().unwrap();
                // Memory Free
                let mem_free = lines[1].split_ascii_whitespace().collect::<Vec<&str>>();
                let mem_free = mem_free[1].parse::<u64>().unwrap();

                let memory_usage = mem_total / mem_free;
                mem_tx.send(memory_usage).unwrap();

                // Cpu Usage
                let mut sys = m_sys.lock().unwrap();
                sys.refresh_cpu();
                let new_avg_cpu_usage: f32 = ((sys
                    .cpus()
                    .iter()
                    .map(sysinfo::CpuExt::cpu_usage)
                    .sum::<f32>())
                    / sys.cpus().len() as f32)
                    .ceil();
                cpu_tx.send(new_avg_cpu_usage).unwrap();
                drop(sys);

                std::thread::sleep(Duration::from_secs(1));

                // Ip Address
                let mut ip_addresses = vec![];
                let network_interfaces = list_afinet_netifas().unwrap();
                for (_, ip) in network_interfaces.iter().filter(|(name, ip)| {
                    args.interface.iter().any(|a| *a == *name) && matches!(ip, IpAddr::V4(_))
                }) {
                    if !ip_addresses.iter().any(|x| x == &ip.to_string()) {
                        ip_addresses.push(ip.to_string());
                    }
                }

                // create ip addresses string
                let mut ip_addresses_string = "[".to_string();
                for (i, address) in ip_addresses.iter().enumerate() {
                    ip_addresses_string += &address.to_string();

                    if i != ip_addresses.len() - 1 {
                        ip_addresses_string += ", ";
                    }
                }
                ip_addresses_string += "]";
                ip_addresses_tx.send(ip_addresses_string).unwrap();
            }
        });

        // X updater thread
        x.spawn(move || {

            // Status string
            let mut last_bat0 = String::new();
            let mut last_bat1 = String::new();
            let mut last_mem_usage = 0;
            let mut last_cpu_usage = 0.0;
            let mut last_addrs = String::new();

            let mut status = String::new();

            loop {
                status.clear();
                // Get the time and make the status message
                let local: DateTime<Local> = Local::now();

                // Battery
                let mut battery_s = String::new();
                if let Ok(bat0) = bat0_rx.try_recv() && battery_00_enable {
                    last_bat0 = bat0.clone();
                }
                if !last_bat0.is_empty() {
                    battery_s.push_str(&format!("{last_bat0}%"));
                }
                if let Ok(bat1) = bat1_rx.try_recv() && battery_01_enable {
                    last_bat1 = bat1.clone();
                }
                if !last_bat1.is_empty() {
                    battery_s.push_str(&format!(", {last_bat1}%"));
                }
                let battery_s = if battery_s.is_empty() {
                    String::new()
                } else {
                    format!(" bat [{battery_s}],")
                };

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

                write!(
                    status,
                    "[{sys_host_name}][{sys_user_name}] => cpu {last_cpu_usage:02}%, mem {last_mem_usage:02}%, net {last_addrs},{battery_s} {}",
                    local.format("%F %T")
                )
                .unwrap();

                // Write and flush the status
                let _ = Command::new("xsetroot")
                    .args(["-name", &status])
                    .status()
                    .unwrap();

                std::thread::sleep(Duration::from_secs(1));
            }
        });
    });
}
