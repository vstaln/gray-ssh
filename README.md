# gray-ssh

`ssh_run` / `ssh_copy` tools delegating shell commands and file copies to remote hosts over SSH/SCP. Port of pi's `ssh` extension.

A sidecar plugin for [gray](https://github.com/vstaln/gray), scaffolded by
[gray-account](https://github.com/vstaln/gray-account).

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

When `~/.gray/ssh/hosts` exists, only the hosts listed there (one per line,
`#` comments ok) may be used. No file → any host.

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
