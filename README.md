# zkSync State Reconstruction Tool

> Tool to reconstruct zkSync state from commit blocks published on L1.

## Table of contents

<!--toc:start-->
- [zkSync State Reconstruction Tool](#zksync-state-reconstruction-tool)
  - [Prerequisites & setup](#prerequisites--setup)
  - [Usage](#usage)
    - [Reconstruction](#reconstruction)
    - [Snapshots](#snapshots)
      - [Generating snapshots](#generating-snapshots)
      - [Importing snapshots](#importing-snapshots)
    - [Additional commands](#additional-commands)
<!--toc:end-->

## Prerequisites & setup

Currently there are three ways to setup the environment: using the provided Nix flake, the container image, or installing the dependencies manually.

<details>
<summary>Nix Flake (Linux only)</summary>

To use the supplied Nix development environment you need to have Nix installed. This can be done by following the official instructions <a href="https://nixos.org/download.html">here</a>.   <br><br>

Once Nix is installed, the development environment can be activated via the following command:

```nix
nix develop --experimental-features 'nix-command flakes'
```

If you instead want to permanently enable the experimental flakes feature, you can do so by following the instructions detailed <a href="https://nixos.wiki/wiki/Flakes">here</a>. The environment can then be activated via:

```nix
nix develop
```

</details>

<details>
<summary>Container Image</summary>
To build the container image, use:
<br><br>

```fish
podman build -t state-reconstruction:latest .
```

And, to run it with `podman`, please use:

```fish
podman run -it state-reconstruction:latest
```

</details>

<details>
<summary>Manually</summary>
This tool is written in nightly Rust; you can install Rust by following the official instructions <a href="https://www.rust-lang.org/learn/get-started">here</a>, and then running the following command to switch to the nightly toolchain:
<br><br>

```fish
rustup toolchain install nightly
```

You also need to have `protobuf`, version `3.20` or above, installed and accessible via `PATH`. Use your preferred package manager to do this. For example, using brew:

```fish
brew install protobuf
```

</details>

> [!IMPORTANT]
> It is highly recommend to override the maximum number of open file descriptors. Without doing so you may eventually run into an error, halting progress. On Unix machines this can be done by using `ulimit` along with the `-n` argument:
>
> ```fish
> ulimit -n 8192
> ```

## Usage

### Reconstruction

To start reconstructing the state, run the following command with any valid HTTP/HTTPS Ethereum JSON-RPC endpoint, for example using `https://eth.llamarpc.com`:

```fish
cargo run --release -- reconstruct l1 --http-url https://eth.llamarpc.com
```

Once the tool is running it will continuously output the state reconstruction progress in the following format:

```fish
2024-01-02T13:29:45.351733Z  INFO No existing database found, starting from genesis...
2024-01-02T13:29:46.028250Z  INFO PROGRESS: [ 0%] CUR BLOCK L1: 16627460 L2: 0 TOTAL BLOCKS PROCESSED L1: 0 L2: 0
2024-01-02T13:29:56.030022Z  INFO PROGRESS: [ 0%] CUR BLOCK L1: 16636036 L2: 11 TOTAL BLOCKS PROCESSED L1: 8451 L2: 11
2024-01-02T13:30:06.031946Z  INFO PROGRESS: [ 0%] CUR BLOCK L1: 16644868 L2: 27 TOTAL BLOCKS PROCESSED L1: 16378 L2: 27
```

<details>
<summary>Metrics reference</summary>

- `CUR BLOCK`: The last block height that was processed.
- `TOTAL BLOCKS PROCESSED`: The total number of blocks that has been processed since starting.

</details>

On each block insert, the tool will compare the new state root hash with that published on L1. Should they differ, the tool will panic. You can then use the `query` command to get additional information, as such:

```fish
cargo run --release -- query root-hash

Batch: <BATCH NUMBER where hash deviated>
Root Hash: <ROOT HASH of the local state tree>
```

### Snapshots

Additionally, the state reconstruction tool provides ways to interact with the upcoming [zkSync Era](https://github.com/matter-labs/zksync-era) snapshot system.

#### Generating snapshots

Before being able to generate snapshots, it is necessary to first fetch the data from L1 and process it. This can be done by running the following command which will also show progress similarly to when reconstructing state from L1.

```fish
cargo run --release -- prepare-snapshot --http-url https://eth.llamarpc.com
```

Once the tool has gathered a number of storage logs, snapshots can then be exported by using the following command, specifying where to export the snapshot directory to.

```fish
cargo run --release -- export-snapshot <DIRECTORY>
```

#### Importing snapshots

Snapshots can also be imported directly to use as a base when reconstructing the state from L1. This can be done by using the `--snapshot <DIRECTORY>` argument when starting reconstruction, like so:

> [!WARNING]
> Importing a snapshot when reconstruction has progressed further than the snapshot has the same effect as truncating the reconstructed state to that of the snapshot.

```fish
cargo run --release -- reconstruct --snapshot <DIRECTORY> l1 --http-url <HTTP-URL>
```

### Additional commands

To view all available options, you can use the `help` command:

```fish
cargo run --release -- --help

zkSync state reconstruction tool

Usage: state-reconstruct <COMMAND>

Commands:
  download          Download L2 state from L1 to JSON file
  reconstruct       Reconstruct L2 state from a source
  query             Query the local storage, and optionally, return a JSON-payload of the data
  prepare-snapshot
  export-snapshot
  help              Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

You can also view all the options for the subcommands in a similar fashion:

```fish
cargo run --release -- reconstruct --help

Reconstruct L2 state from a source

Usage: state-reconstruct reconstruct [OPTIONS] <COMMAND>

Commands:
  l1    Fetch data from L1
  file  Fetch data from a file
  help  Print this message or the help of the given subcommand(s)

Options:
  -d, --db-path <DB_PATH>    The path to the storage solution [env: ZK_SYNC_DB_PATH=]
      --snapshot <SNAPSHOT>  If present, try to restore state from snapshot files contained in the specified directory. Note that this will only work when supplied with a fresh database
  -h, --help                 Print help
```
