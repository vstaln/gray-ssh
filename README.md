<p align="center">
  <img src="assets/gray-logo.svg" alt="gray" width="96">
</p>
<h1 align="center">gray-ssh</h1>
<p align="center">Run commands and copy files on remote hosts over SSH/SCP.</p>
<p align="center">
  <a href="https://github.com/vstaln/gray-ssh/blob/main/LICENSE"><img alt="MIT License" src="https://img.shields.io/badge/license-MIT-blue.svg"></a>
  <img alt="gray plugin" src="https://img.shields.io/badge/gray-plugin-7aa2f7.svg">
  <img alt="rust" src="https://img.shields.io/badge/built%20with-rust-orange.svg">
</p>

`ssh_run` / `ssh_copy` tools delegate shell commands and file copies to remote hosts over SSH/SCP, with an optional host allowlist.

## What it does

- `ssh_run {host, command, timeout_secs?}` →
  `ssh -o BatchMode=yes -o ConnectTimeout=8 <host> <command>`.
  Combined stdout+stderr capped at 32KB; nonzero exit or timeout → is_error.
  Default timeout 60s.
- `ssh_copy {host, local, remote, direction:"up"|"down", timeout_secs?}` →
  `scp` in the matching direction. Default timeout 300s.

Requires key-based auth — `BatchMode=yes` means no password prompts are
possible. Hosts, paths, and commands that start with `-` are refused so they
can't smuggle options to ssh/scp.

### Host allowlist

Deny by default. Only the hosts listed in `~/.gray/ssh/hosts` (one per line,
`#` comments ok) may be used. Until that file exists, every host is refused.

## Wire methods

`plugin/manifest`, `tool/call`, `plugin/shutdown`.

## Install

```sh
gray plugin install ssh
```

## Develop

```sh
cargo test
cargo build --release
gray account check      # entry point + manifest handshake
```

---
Part of the [gray](https://github.com/vstaln/gray) plugin ecosystem —
the open-source AI agent harness. <https://gray.alignment.id>
