# Architecture

```
                 ┌────────────────────────────────────────────────────────────┐
   user apps ──► │ TUN (10.10.0.2/24) ──┐                                    │
                 │                      ▼                                    │
                 │           ┌──────────────────────┐                        │
                 │           │  Leaf #2 (main)      │                        │
                 │           │  - tun inbound       │                        │
                 │           │  - chain outbound:   │                        │
                 │           │     1) socks-arti    │ ─────► Arti SOCKS  ─┐  │
                 │           │     2) vless-reality │                    │  │
                 │           └──────────────────────┘                    │  │
                 │                      ▲                                ▼  │
                 │ embedded DNS @ 10.10.0.2:53 ──── DoH POST ──── via Leaf#2 │
                 │                                                            │
                 │                                                  Arti (Tor)│
                 │                                                  + bridge  │
                 │                                                  + PT proxy│
                 │                                                            │
                 │                                                  ▲         │
                 │                                                  │         │
                 │                                ┌─────────────────┘         │
                 │                                │                           │
                 │                       ┌────────────────┐                   │
                 │                       │  Leaf #1 (PT)  │                   │
                 │                       │  - socks5 in   │ ◄─── Arti dials   │
                 │                       │  - direct out  │      bridge       │
                 │                       └────────────────┘                   │
                 └────────────────────────────────────────────────────────────┘
```

## Threading

- One Tokio multi-thread runtime owned by `ff_vpn_core::rt()`.
- Leaf instances run on `spawn_blocking` threads (Leaf has its own internal
  multi-threaded scheduler — we do not assume cooperative scheduling between
  it and Tokio).
- Arti runs on `spawn_blocking` (CLI entrypoint blocks).
- DNS proxy + IPC server are normal Tokio tasks.
- **All** of them watch a single `Cancel` token; cancelling stops everything
  in well under a second.

## Why two Leaf instances?

Leaf is the SOCKS provider for the Pluggable Transport (Leaf #1) *and* the
TUN→VLESS engine that carries the user's traffic (Leaf #2). They have very
different configs and lifetimes, and Leaf does not multiplex multiple
"profiles" inside a single runtime — using two runtimes keeps the
responsibilities and shutdown paths clean.

## Future-proofing the PT layer

`crate::pt::PluggableTransport` is the interface the engine talks to. To swap
Leaf out for Lyrebird or Xray-core, add a new module under `pt/`,
implement `PluggableTransport`, and wire it into `pt::spawn`. No engine
changes are needed.
