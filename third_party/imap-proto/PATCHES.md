# Lightmail compatibility patch

Based on the crates.io source of `imap-proto 0.16.7` from
<https://github.com/djc/imap-proto>. The original MIT / Apache-2.0 licenses remain
in this directory.

QQ Mail can return `NIL` in the `BODYSTRUCTURE` transfer-encoding field of a
message. The upstream parser requires a string and rejects the entire FETCH
response, which prevents both batch summary synchronization and body reading.

The only parser change accepts `NIL` in `body_encoding` and maps it to `SevenBit`,
the default for a missing Content-Transfer-Encoding header under
[RFC 2045 section 6.1](https://www.rfc-editor.org/rfc/rfc2045#section-6.1).
The parser still validates sizes and other fields; existing encodings remain
unchanged. No message bytes are rewritten.

Regression coverage lives in `core/tests.rs` and `tests/mail_fixture.py` and
runs through `python3 scripts/check.py`. Test data is synthetic. Remove this
local override when an upstream version includes equivalent compatibility.
