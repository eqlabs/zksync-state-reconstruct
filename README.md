# zksync-state-reconstruct
> Tool / Library to reconstruct zkSync state from commit blocks

## Prerequisites
This tool is written in nightly Rust; you can install Rust by following the official instructions [here](https://www.rust-lang.org/learn/get-started), and then running the following command to switch to the nightly toolchain:

```fish
rustup toolchain install nightly
```

## Usage
To start reconstructing the state, run the following command with any valid HTTP/HTTPS Ethereum JSON-RPC endpoint, for example using `https://eth.llamarpc.com`:

```fish
cargo +nightly run -- reconstruct l1 --http-url https://eth.llamarpc.com
```

To view all available options, you can use the `help` command:

```fish
$ cargo +nightly run -- --help

zkSync state reconstruction tool

Usage: state-reconstruct <COMMAND>

Commands:
  reconstruct  Reconstruct L2 state from a source
  query        Query the local storage, and optionally, return a JSON-payload of the data
  help         Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

You can also view all the options for the subcommands in a similar fashion:

```fish
$ cargo +nightly run -- reconstruct --help

Reconstruct L2 state from a source

Usage: state-reconstruct reconstruct [OPTIONS] <COMMAND>

Commands:
  l1    Fetch data from L1
  file  Fetch data from a file
  help  Print this message or the help of the given subcommand(s)

Options:
  -d, --db-path <DB_PATH>  The path to the storage solution [env: ZK_SYNC_DB_PATH=]
  -h, --help               Print help
```
