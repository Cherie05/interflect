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
open from an untrusted source. The parser is written to fail cleanly rather than
crash — malformed input produces a diagnostic and exit code 1, never a panic,
and this is covered both by a unit test and by a CI gate.

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

Please include the `.rad` file or input that triggers it, the version
(`interflect --help` prints it) and your OS.

You can expect an acknowledgement within a week. Given the scope above, most
findings will be treated as ordinary bugs and fixed in the open — but the
private channel exists so that call is made after seeing the report, not before.
