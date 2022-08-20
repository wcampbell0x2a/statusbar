#![feature(string_remove_matches)]

use chrono::{DateTime, Local};
use local_ip_address::list_afinet_netifas;
use std::fmt::Write;
use std::net::IpAddr;
use std::sync::{mpsc::channel, Arc, Mutex};
use std::time::Duration;
use sysinfo::{CpuExt, System, SystemExt, UserExt};

#[link(name = "X11")]
extern "C" {
    fn XOpenDisplay(screen: usize) -> usize;
    fn XStoreName(display: usize, window: usize, name: *const u8) -> i32;
    fn XDefaultRootWindow(display: usize) -> usize;
    fn XFlush(display: usize) -> i32;
}

fn main() {
    let mut ip_addresses = vec![];
    let (bat0_tx, bat0_rx) = channel();
    let (bat1_tx, bat1_rx) = channel();
    let (mem_tx, mem_rx) = channel();
    let (cpu_tx, cpu_rx) = channel();
    let m_sys = Arc::new(Mutex::new(System::new_all()));

    // First call to sys functions, grabbing host_name and user name, and also ip addresses
    let (sys_host_name, sys_user_name) = {
        let mut sys = m_sys.lock().unwrap();

        sys.refresh_all();

        let network_interfaces = list_afinet_netifas().unwrap();
        for (_, ip) in network_interfaces.iter().filter(|(name, ip)| {
            *name == "wlan0" || *name == "enp0s31f6" && matches!(ip, IpAddr::V4(_))
        }) {
            ip_addresses.push(ip.to_string());
        }

        (sys.host_name().unwrap(), sys.users()[1].name().to_string())
    };

    // create ip addresses string
    let mut ip_addresses_string = "[".to_string();
    for (i, address) in ip_addresses.iter().enumerate() {
        ip_addresses_string += &address.to_string();

        if i != ip_addresses.len() - 1 {
            ip_addresses_string += ", ";
        }
    }
    ip_addresses_string += "]";

    // Thread updating every n seconds
    std::thread::scope(|x| {
        x.spawn(move || {
            loop {
                // Battery 0
                let mut bat0 =
                    std::fs::read_to_string("/sys/class/power_supply/BAT0/capacity").unwrap();
                bat0.remove_matches('\n');
                bat0_tx.send(bat0).unwrap();

                // Battery 1
                let mut bat1 =
                    std::fs::read_to_string("/sys/class/power_supply/BAT1/capacity").unwrap();
                bat1.remove_matches('\n');
                bat1_tx.send(bat1).unwrap();

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
                let new_avg_cpu_usage: f32 =
                    ((sys.cpus().iter().map(|a| a.cpu_usage()).sum::<f32>())
                        / sys.cpus().len() as f32)
                        .ceil();
                cpu_tx.send(new_avg_cpu_usage).unwrap();

                std::thread::sleep(Duration::from_secs(1));
            }
        });

        // Connect to X
        let disp = unsafe { XOpenDisplay(0) };
        let root = unsafe { XDefaultRootWindow(disp) };

        // X updater thread
        x.spawn(move || {

            // Status string
            let mut last_bat0 = String::new();
            let mut last_bat1 = String::new();
            let mut last_mem_usage = 0;
            let mut last_cpu_usage = 0.0;

            let mut status = String::new();

            loop {
                status.clear();
                // Get the time and make the status message
                let local: DateTime<Local> = Local::now();
                if let Ok(bat0) = bat0_rx.try_recv() {
                    last_bat0 = bat0.clone();
                }
                if let Ok(bat1) = bat1_rx.try_recv() {
                    last_bat1 = bat1.clone();
                }
                if let Ok(mem_usage) = mem_rx.try_recv() {
                    last_mem_usage = mem_usage;
                }
                if let Ok(cpu_usage) = cpu_rx.try_recv() {
                    last_cpu_usage = cpu_usage;
                }
                write!(
                    status,
                    "[{sys_host_name}][{sys_user_name}] => cpu {last_cpu_usage}%, mem {last_mem_usage}%, net {ip_addresses_string}, bat [{last_bat0}%, {last_bat1}%], {}\0",
                    local.format("%F %T")
                )
                .unwrap();

                // Write and flush the status
                unsafe {
                    XStoreName(disp, root, status.as_ptr());
                }
                unsafe {
                    XFlush(disp);
                }

                //std::thread::sleep(Duration::from_nanos((1e9 / 144.) as u64));
                std::thread::sleep(Duration::from_secs(1));
            }
        });
    });
}
