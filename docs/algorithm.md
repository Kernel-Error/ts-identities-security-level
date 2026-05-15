# Algorithm

## Identity file format

A TeamSpeak 3 identity is stored as a small `.ini`:

```ini
[Identity]
id=<local id string>
identity="<counter>V<obfuscated_blob_base64>"
nickname=<nickname>
phonetic_nickname=<phonetic>
```

The cryptographically relevant field is `identity="…"`. Inside the quotes:

- `counter` — decimal ASCII of a `uint64`. The proof-of-work nonce. Hashing
  raises the security level by finding a value of `counter` that produces
  a SHA-1 digest with more leading zero bits.
- `V` — literal separator.
- `obfuscated_blob_base64` — base64 of XOR-obfuscated ASCII; the ASCII,
  base64-decoded a second time, yields the libtomcrypt ASN.1 DER of an
  ECDSA P-256 keypair (`flags = 1` → private+public).

## Security level formula

```
level = leading_zero_bits( SHA1( pubkey_b64 || decimal(counter) ) )
```

Where:

- `pubkey_b64` is the base64 of the **public-only** libtomcrypt ASN.1 DER
  (the same string the TS3 client shows as "Public Key").
- `decimal(counter)` is `format!("{}", counter)`, no padding.
- Concatenation is byte-level on the two ASCII strings.

Bit counting follows the hashcat / TeamSpeakHasher convention: count zero
bytes from byte 0 upward; within the first non-zero byte, count trailing
zeros (LSB first):

```python
zero_bytes = 0
while zero_bytes < 20 and hash[zero_bytes] == 0:
    zero_bytes += 1
zero_bits = 0
if zero_bytes < 20:
    b = hash[zero_bytes]
    while not (b & 1):
        zero_bits += 1
        b >>= 1
level = 8 * zero_bytes + zero_bits
```

The reference implementation lives at
[`crates/ts3level-core/src/level.rs`](../crates/ts3level-core/src/level.rs).
The GPU kernel is verified against it byte-for-byte in
[`crates/ts3level-cuda/tests/cpu_parity.rs`](../crates/ts3level-cuda/tests/cpu_parity.rs).

## De-obfuscation of the keypair blob

Two-step XOR mask, symmetric (applying the routine twice is a no-op).
Source: `landave/TSIdentityTool.c`.

```
data = base64_decode(blob_b64)        # ~248 bytes
hash_part = data[20..]                # up to first NUL byte (C-string semantics)
sha = sha1(hash_part)
data[0..20] ^= sha                    # SHA-1 mask
data[0..min(100, len)] ^= TSKEY       # static XOR key
inner_ascii_b64 = data                # NUL-terminated base64
asn1_der = base64_decode(inner_ascii_b64)
```

`TSKEY` is the 128 ASCII bytes of the hex string

```
b9dfaa7bee6ac57ac7b65f1094a1c155e747327bc2fe5d51c512023fe54a2802
01004e90ad1daaae1075d53b7d571c30e063b5a62a4a017bb394833aa0983e6e
```

(not the 64 binary bytes those hex chars decode to). The Rust constant
lives in
[`crates/ts3level-core/src/deobfuscate.rs`](../crates/ts3level-core/src/deobfuscate.rs).

## libtomcrypt keypair DER

```
SEQUENCE {
  BIT STRING       flags     -- 1 bit; 0 = public-only, 1 = with private k
  SHORT INTEGER    keysize   -- 32 for the NIST P-256 curve
  INTEGER          x
  INTEGER          y
  [INTEGER         k]        -- private scalar; only when flags = 1
}
```

`pubkey_b64` is base64 of the same SEQUENCE with `flags = 0` and the `k`
INTEGER omitted. The reader/re-emitter is
[`crates/ts3level-core/src/pubkey.rs`](../crates/ts3level-core/src/pubkey.rs).

## Updating the level

The only thing that changes when the level is raised is the digits before
the `V` in the `identity=` value. The obfuscated blob stays byte-identical;
the public key never changes. `set_counter` +
[`writer::write_back`](../crates/ts3level-core/src/writer.rs) preserve
everything else byte-for-byte and replace the file atomically.

## Provenance

The format, the obfuscation key, and the level definition were not lifted
from TeamSpeak's source code or shipped binaries. They are derived from
public, long-standing third-party documentation built by black-box analysis
of the `.ini` file format and observable behavior of the client:

- [`landave/TSIdentityTool`](https://github.com/landave/TSIdentityTool) —
  C reference implementation, includes the XOR key and the level routine.
- [`landave/TeamSpeakHasher`](https://github.com/landave/TeamSpeakHasher) —
  OpenCL hasher, source of the bit-counting convention.
- [`ReSpeak/tsdeclarations`](https://github.com/ReSpeak/tsdeclarations) —
  protocol notes including the obfuscation steps.
- [`hashcat/hashcat`](https://github.com/hashcat/hashcat) — the
  `OpenCL/inc_hash_sha1.h` round and step macros (MIT-licensed) inform
  the shape of our CUDA SHA-1 inner loop (LOP3-friendly Ch/Maj forms,
  funnelshift rotates, the 5-step unrolled register cycle).

This project's Rust implementation was written from those references, not
copied. No TeamSpeak source code, header, or binary was used or
distributed.
