statusbar
===========================

[<img alt="github" src="https://img.shields.io/badge/github-wcampbell0x2a/statusbar-8da0cb?style=for-the-badge&labelColor=555555&logo=github" height="20">](https://github.com/wcampbell0x2a/statusbar)
[<img alt="crates.io" src="https://img.shields.io/crates/v/statusbar.svg?style=for-the-badge&color=fc8d62&logo=rust" height="20">](https://crates.io/crates/statusbar)
[<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-statusbar-66c2a5?style=for-the-badge&labelColor=555555&logo=docs.rs" height="20">](https://docs.rs/statusbar)
[<img alt="build status" src="https://img.shields.io/github/actions/workflow/status/wcampbell0x2a/statusbar/main.yml?branch=master&style=for-the-badge" height="20">](https://github.com/wcampbell0x2a/statusbar/actions?query=branch%3Amaster)

A DWM statusbar that shows the following information:
```
[hostname][user] => cpu %, mem %, net [ip0, ip1], bat [bat0, bat1], date time
```

## install
`cargo install statusbar` or see our github [releases](https://github.com/wcampbell0x2a/statusbar/releases).

## usage
```
Usage: statusbar [OPTIONS]

Options:
      --interface <INTERFACE>  network interface for display of ip addresses
      --username <USERNAME>    override return from first user in sys.users()
  -h, --help                   Print help information
  -V, --version                Print version information
```

For example:
```
./statusbar --interface wlan0 --interface enp0s31f6
```
