# Security Policy

## Supported versions

| Version | Supported |
|---|---|
| 0.1.x | yes |

## Threat model

Interflect is an offline command-line renderer. It reads a scene file and writes
a PNG. It opens no network connections, executes nothing it reads, and requires
no elevated privileges.

The realistic attack surface is **a malicious `.rad` file**, which someone might
open from an untrusted source. Malformed input produces a diagnostic and exit
code 1 rather than a panic, and this is covered by unit tests and a CI gate.

Settings in the `render {}` block are bounded, because they are read straight
from the scene file and reach allocation sizes directly. An audit found
`surfels: 999999999` aborting the process on a 76 GB allocation and a
200000x200000 film hanging indefinitely; both are now rejected with a message.
The same limits are re-checked after CLI overrides, since `--surfels` bypasses
the parser. Current ceilings are in `scene.rs`: 16384 px per side, 40M pixels,
2M surfels, 4096 bounces.

Things worth reporting:

- A `.rad` file that causes a panic, an out-of-bounds access, or unbounded
  memory or CPU consumption
- Any path traversal through `-o` or a scene reference
- A crash reachable from a PNG the `compare` tool reads

There is no `unsafe` code in this repository.

## Reporting a vulnerability

Use GitHub's private reporting: **Security → Report a vulnerability** on
[the repository](https://github.com/Cherie05/interflect/security). That keeps
the report confidential until a fix ships.

If you would rather not use GitHub, email <arunvpp24@gmail.com> instead.

Please include the `.rad` file or input that triggers it, the version
(`interflect --help` prints it) and your OS.

You can expect an acknowledgement within a week. Given the scope above, most
findings will be treated as ordinary bugs and fixed in the open — but the
private channel exists so that call is made after seeing the report, not before.
