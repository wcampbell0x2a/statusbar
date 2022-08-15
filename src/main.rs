#![feature(string_remove_matches)]

use chrono::{DateTime, Local};
use std::fmt::Write;
use std::sync::mpsc::channel;
use std::time::Duration;

#[link(name = "X11")]
extern "C" {
    fn XOpenDisplay(screen: usize) -> usize;
    fn XStoreName(display: usize, window: usize, name: *const u8) -> i32;
    fn XDefaultRootWindow(display: usize) -> usize;
    fn XFlush(display: usize) -> i32;
}

fn main() {
    let (bat0_tx, bat0_rx) = channel();
    let (bat1_tx, bat1_rx) = channel();
    let (mem_tx, mem_rx) = channel();

    std::thread::scope(|x| {
        // Second increment update
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
                let mem_total = u64::from_str_radix(mem_total[1], 10).unwrap();
                // Memory Free
                let mem_free = lines[1].split_ascii_whitespace().collect::<Vec<&str>>();
                let mem_free = u64::from_str_radix(mem_free[1], 10).unwrap();

                let memory_usage = mem_total / mem_free;
                mem_tx.send(memory_usage).unwrap();

                // Cpu Usage

                std::thread::sleep(Duration::from_secs(1));
            }
        });

        // X updater thread
        x.spawn(move || {
            // Connect to X
            let disp = unsafe { XOpenDisplay(0) };
            let root = unsafe { XDefaultRootWindow(disp) };

            // Status string
            let mut last_bat0 = String::new();
            let mut last_bat1 = String::new();
            let mut last_mem_usage = 0;

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
                    last_mem_usage = mem_usage.clone();
                }
                write!(
                    status,
                    "mem {last_mem_usage}%, bat [{last_bat0}%, {last_bat1}%], {}\0",
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

                std::thread::sleep(Duration::from_nanos((1e9 / 144.) as u64));
            }
        });
    });
}
