<div align="center">

# microvm

**Firecracker microVMs in seconds.**<br> No kernels to build. No TAP devices to
wire. No SSH keys to juggle.

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![crates.io](https://img.shields.io/crates/v/taumaru-microvm.svg)](https://crates.io/crates/taumaru-microvm)
[![CI](https://github.com/Taumaru/taumaru-microvm/actions/workflows/ci.yml/badge.svg)](https://github.com/Taumaru/taumaru-microvm/actions/workflows/ci.yml)

</div>

```console
$ curl -fsSL https://microvm.taumaru.com/install.sh | sh
$ microvm new dev          # pulls Firecracker, a kernel, and Ubuntu, then builds the VM
$ microvm start dev        # boots the VM
$ microvm ssh dev          # you are root in a real, KVM-isolated Linux machine
```

Firecracker is the VMM behind AWS Lambda and Fargate: hardware isolation with
the startup speed of a container. It is also a bare-metal tool. Before your
first boot you have to find a compatible kernel, build a root filesystem, create
TAP devices and routes, assign IP addresses, inject SSH keys, write JSON config,
and keep track of every process and socket yourself.

**`microvm` does all of that for you.** You get persistent virtual machines that
behave like real servers: create them, start them, SSH into them, snapshot them
while they run, move them to another host, and have them come back up when the
host reboots.

## Why microvm

- **From zero to SSH in three commands.** `microvm new` downloads the
  Firecracker runtime, a tuned Linux kernel, and a ready-to-boot distribution
  image from the curated
  [Taumaru Artifacts Registry](https://artifacts.taumaru.com/v1/registry.json).
  Every file is checked against its SHA-256 digest and cached for later VMs.
- **Real machines, not throwaway sandboxes.** Each VM gets its own ext4 disk
  sized to your request, its own ed25519 key pair injected at creation, and its
  own network identity. It keeps its state across stops, starts, and host
  reboots.
- **SSH with nothing to set up.** `microvm ssh dev` opens a shell and
  `microvm ssh dev -- make test` runs a command. Keys, addresses, and ports are
  handled for you.
- **Networking that just works.** VMs are host-only by default. Add
  `--expose-lan` and the VM gets a routed address on your local network.
- **Live, portable snapshots.** Snapshot a _running_ VM into a single
  `.tmvmsnap` file with no downtime. A Device Mapper point-in-time view captures
  the disk while the guest keeps running. Restore it on the same host or a
  different one, and choose whether to keep or reassign its IP addresses.
- **Autostart at boot.** Mark a VM to start with the host through a systemd
  unit, with retry attempts and pause/resume.
- **No daemon, no stale state.** There is no background service. A VM's state is
  checked live against its Firecracker control socket every time you ask. The
  inventory is a local SQLite database.
- **Made for humans and scripts.** Guided prompts when you are at a terminal,
  and `--non-interactive` with explicit flags for automation. Every error tells
  you what happened, why, and what to do next.
- **A library too.** The CLI is a thin layer over the
  [`taumaru-microvm`](crates/sdk/README.md) Rust SDK. Anything the CLI does,
  your own program can do.

### Firecracker by hand vs. microvm

| Task                             | Firecracker by hand                                    | microvm                                                      |
| -------------------------------- | ------------------------------------------------------ | ------------------------------------------------------------ |
| Get a kernel and root filesystem | Build or hunt for compatible ones                      | Downloaded and verified automatically                        |
| Networking                       | `ip tuntap`, routes, NAT, IP bookkeeping               | Host-only or `--expose-lan`, allocated for you               |
| SSH access                       | Bake keys into the image yourself                      | Per-VM ed25519 key injected; `microvm ssh`                   |
| Start / stop                     | Write the JSON config, manage the socket and PID       | `microvm start` / `microvm stop`                             |
| Know what is running             | Track processes yourself                               | `microvm ls`, checked live                                   |
| Backup and migration             | Copy the disk while the VM is down, rebuild the config | `microvm snapshot` while it runs, `microvm restore` anywhere |
| Survive host reboot              | Write your own systemd units                           | `microvm autostart add`                                      |

## Quickstart

### 1. Check the requirements

- Linux on **x86_64** with KVM available (`ls /dev/kvm`). Bare metal and nested
  virtualization both work.
- `sudo` (or `pkexec`). Firecracker, networking, and Device Mapper need root.
  `microvm` asks for elevation only when a command needs it.
- An `ssh` client.

### 2. Install

```sh
curl -fsSL https://microvm.taumaru.com/install.sh | sh
```

The installer downloads the static binary for your architecture, checks its
SHA-256 digest, and puts `microvm` in `~/.local/bin`. It adds that directory to
your `PATH` if needed.

<details>
<summary>Installer options</summary>

| Variable                             | Effect                                           |
| ------------------------------------ | ------------------------------------------------ |
| `TAUMARU_MICROVM_VERSION=v0.3.0`     | Install a specific release instead of the latest |
| `TAUMARU_INSTALL_DIR=/usr/local/bin` | Choose the install directory                     |
| `TAUMARU_NO_MODIFY_PATH=1`           | Do not edit shell profile files                  |

</details>

### 3. Create your first microVM

```sh
microvm new dev
```

`new` walks you through the image, vCPUs, memory, and disk size, then downloads
whatever is missing and builds the VM. To skip the prompts, pass everything as
flags:

```sh
microvm new dev --image ubuntu-24.04=ubuntu-24.04 --vcpus 2 --memory 1GB --disk-gb 10
```

> [!TIP] Want Docker inside the VM? Use
> `--image ubuntu-24.04=ubuntu-24.04-docker`.

### 4. Boot it and log in

```sh
microvm start dev
microvm ssh dev
```

That's it. You are `root` on Ubuntu 24.04, inside a KVM-isolated microVM. To run
one command and return:

```sh
microvm ssh dev -- uname -a
```

### 5. Look around, then clean up

```sh
microvm ls            # every VM with its state, vCPUs, memory, and disk
microvm stop dev
microvm delete dev    # removes the VM, its disk, and its network resources
```

Kernels and images stay cached, so your next `microvm new` is even faster.

## Commands

Commands that take a VM name open an interactive picker when you leave the name
out.

| Command                               | What it does                                                                |
| ------------------------------------- | --------------------------------------------------------------------------- |
| `microvm new [NAME]`                  | Create a VM (`--image`, `--vcpus`, `--memory`, `--disk-gb`, `--expose-lan`) |
| `microvm start [NAME]`                | Boot a VM                                                                   |
| `microvm stop [NAME]`                 | Shut a VM down                                                              |
| `microvm ssh [NAME] [-- CMD...]`      | Open a shell, or run a command inside a running VM                          |
| `microvm ls`                          | List VMs with live state and configured capacity (alias: `list`)            |
| `microvm snapshot [NAME] [OUTPUT]`    | Write a portable `.tmvmsnap` archive, even from a running VM                |
| `microvm restore [ARCHIVE]`           | Recreate a VM from a snapshot archive                                       |
| `microvm delete [NAME]`               | Delete a VM and everything it owns                                          |
| `microvm autostart add\|edit\|rm\|ls` | Control which VMs start when the host boots                                 |
| `microvm artifacts download`          | Pre-fetch the runtime, kernels, and images                                  |
| `microvm artifacts prune`             | Delete kernels and images no VM uses                                        |

Run `microvm <command> --help` for every flag.

## Snapshots and migration

```sh
# On host A, while "dev" keeps running:
microvm snapshot dev ./dev.tmvmsnap --address-policy regenerate

# On host B:
microvm restore ./dev.tmvmsnap
microvm start dev
```

- `--address-policy preserve` keeps the VM's IPv4 addresses, which suits a
  restore on the same network. `regenerate` assigns fresh addresses on the
  destination host.
- A snapshot captures the disk at a point in time, like pulling the power cord.
  It does not capture guest memory. Stop the VM first if you need an
  application-consistent copy.
- Archives are **not encrypted**. They contain the VM's disk and SSH private
  key, so protect them like you would the machine itself.

## Networking

| Mode                | How to get it                  | Reachable from                       |
| ------------------- | ------------------------------ | ------------------------------------ |
| Host-only (default) | `microvm new dev`              | The host                             |
| Routed LAN          | `microvm new dev --expose-lan` | Other machines on your local network |

Addresses are allocated automatically and kept for the life of the VM.
`microvm ssh` always knows where to connect.

## Autostart

```sh
microvm autostart add dev --max-attempts 5   # start dev when the host boots
microvm autostart edit dev --pause           # keep the policy, skip dev for now
microvm autostart ls
microvm autostart rm dev                     # the VM itself is kept
```

## Automation

Every command runs unattended with `--non-interactive`. It then never prompts,
requires the values it would have asked for, and fails fast instead of asking
for a password. Run it as root or with sudo rights already granted.

```sh
sudo microvm new ci-runner --non-interactive \
  --image ubuntu-24.04=ubuntu-24.04 --vcpus 4 --memory 4GB --disk-gb 20
sudo microvm start ci-runner --non-interactive
sudo microvm ssh ci-runner --non-interactive -- ./run-tests.sh
```

| Setting                     | Meaning                                                                    |
| --------------------------- | -------------------------------------------------------------------------- |
| `TAUMARU_HOME=/path`        | Use a different home instead of `~/.taumaru-microvm`                       |
| `NO_COLOR=1`                | Disable colored output (it is also disabled when output is not a terminal) |
| Exit code `0` / `1` / `130` | Success / failure / cancelled                                              |

## Where things live

```text
~/.taumaru-microvm/
├── artifacts/{kernels,rootfs}   # verified, shared downloads
├── tools/                       # Firecracker runtime
├── vms/<name>/                  # one directory per VM: rootfs.ext4, ssh/, firecracker.log
├── tmp/                         # snapshot scratch space
└── state/inventory.db           # SQLite inventory
```

## Use it from Rust

The lifecycle engine is available as a library on crates.io:

```sh
cargo add taumaru-microvm
```

See the [SDK README](crates/sdk/README.md) for a full example.

## How it compares

| Project                                                                                                                                | Best at                                                          | Where microvm differs                                                                |
| -------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------- | ------------------------------------------------------------------------------------ |
| [Firecracker](https://github.com/firecracker-microvm/firecracker) + `firectl`                                                          | The raw VMM and a minimal launcher                               | microvm adds images, networking, keys, inventory, snapshots, and autostart around it |
| [firecracker-go-sdk](https://github.com/firecracker-microvm/firecracker-go-sdk)                                                        | Driving one Firecracker process from Go                          | microvm is a full lifecycle manager with an artifact source, as a CLI and a Rust SDK |
| [Weave Ignite](https://github.com/weaveworks/ignite)                                                                                   | Docker-like microVMs (archived, no longer maintained)            | microvm is actively developed                                                        |
| [Flintlock](https://github.com/liquidmetal-dev/flintlock)                                                                              | gRPC microVM service for bare-metal Kubernetes clusters          | microvm needs no daemon and no cluster; one binary on one host                       |
| [firecracker-containerd](https://github.com/firecracker-microvm/firecracker-containerd), [Kata Containers](https://katacontainers.io/) | Running containers and pods inside microVMs                      | microvm gives you persistent machines you SSH into                                   |
| E2B, microsandbox, and similar                                                                                                         | Short-lived sandboxes for running untrusted or AI-generated code | microvm manages long-lived VMs on your own host, with snapshots and boot autostart   |

## Build from source

```sh
git clone https://github.com/Taumaru/taumaru-microvm
cd taumaru-microvm
cargo build --release -p taumaru-microvm-cli   # binary: target/release/microvm
```

## License

Released under the [MIT license](https://opensource.org/licenses/MIT).
