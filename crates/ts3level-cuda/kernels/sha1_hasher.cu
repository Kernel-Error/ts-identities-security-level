// CUDA SHA-1 brute-force kernel for the TeamSpeak 3 identity security
// level. The hash input per attempt is `pubkey_b64 || decimal(counter)`;
// the level is `leading_zero_bits` of the resulting 160-bit SHA-1 digest
// where bits are counted byte-0 first, LSB-first within each byte (the
// hashcat / TS3 convention).
//
// We support a maximum hash-input length of 192 bytes, which covers a
// canonical TS3 pubkey base64 (~104 chars) plus a 20-digit counter and the
// SHA-1 padding/length footer — three 64-byte blocks.
//
// Per launch: each thread iterates `n_per_thread` counters starting at
// `start_counter + tid * n_per_thread`. Results are coalesced into a
// single device-global packed `(level << 56) | counter` slot via
// `atomicMax(unsigned long long*)`.

#include <stdint.h>

#define MAX_INPUT_BYTES   192
#define MAX_INPUT_BLOCKS  3      // 192 / 64

// ---- SHA-1 core --------------------------------------------------------

__device__ __forceinline__ uint32_t rotl(uint32_t x, uint32_t n) {
    return (x << n) | (x >> (32 - n));
}

// Process a single 64-byte block, updating h[0..5].
__device__ void sha1_block(uint32_t h[5], const uint8_t block[64]) {
    uint32_t w[80];

#pragma unroll
    for (int i = 0; i < 16; i++) {
        w[i] = ((uint32_t)block[i*4    ] << 24) |
               ((uint32_t)block[i*4 + 1] << 16) |
               ((uint32_t)block[i*4 + 2] <<  8) |
               ((uint32_t)block[i*4 + 3]      );
    }
#pragma unroll
    for (int i = 16; i < 80; i++) {
        w[i] = rotl(w[i-3] ^ w[i-8] ^ w[i-14] ^ w[i-16], 1);
    }

    uint32_t a = h[0], b = h[1], c = h[2], d = h[3], e = h[4];

#pragma unroll
    for (int i = 0; i < 20; i++) {
        uint32_t f = (b & c) | ((~b) & d);
        uint32_t t = rotl(a, 5) + f + e + 0x5A827999u + w[i];
        e = d; d = c; c = rotl(b, 30); b = a; a = t;
    }
#pragma unroll
    for (int i = 20; i < 40; i++) {
        uint32_t f = b ^ c ^ d;
        uint32_t t = rotl(a, 5) + f + e + 0x6ED9EBA1u + w[i];
        e = d; d = c; c = rotl(b, 30); b = a; a = t;
    }
#pragma unroll
    for (int i = 40; i < 60; i++) {
        uint32_t f = (b & c) | (b & d) | (c & d);
        uint32_t t = rotl(a, 5) + f + e + 0x8F1BBCDCu + w[i];
        e = d; d = c; c = rotl(b, 30); b = a; a = t;
    }
#pragma unroll
    for (int i = 60; i < 80; i++) {
        uint32_t f = b ^ c ^ d;
        uint32_t t = rotl(a, 5) + f + e + 0xCA62C1D6u + w[i];
        e = d; d = c; c = rotl(b, 30); b = a; a = t;
    }

    h[0] += a; h[1] += b; h[2] += c; h[3] += d; h[4] += e;
}

// ---- Level extraction --------------------------------------------------

// Count leading zero bits of the SHA-1 digest, byte-0 first, LSB-first
// within each byte. Equivalent to the reference C code:
//   while bytes[i]==0 i++; level = 8*i + ffs(bytes[i])-1.
__device__ __forceinline__ uint32_t digest_level(const uint32_t h[5]) {
    uint32_t zero_bytes = 0;
#pragma unroll
    for (int word = 0; word < 5; word++) {
        uint32_t v = h[word];
#pragma unroll
        for (int b = 0; b < 4; b++) {
            uint32_t byte = (v >> ((3 - b) * 8)) & 0xff;
            if (byte == 0) {
                zero_bytes++;
            } else {
                // __ffs returns 1-indexed LSB position, 0 if v==0.
                return zero_bytes * 8 + (uint32_t)__ffs(byte) - 1;
            }
        }
    }
    return 160;
}

// ---- ASCII decimal of a uint64 -----------------------------------------

// Write `n` as decimal ASCII into `out`. Returns the number of digits
// written (1..=20). Output is in normal big-endian / left-to-right order.
__device__ __forceinline__ int utoa64_be(uint64_t n, uint8_t* out) {
    if (n == 0) {
        out[0] = '0';
        return 1;
    }
    uint8_t buf[20];
    int len = 0;
    while (n > 0) {
        buf[len++] = (uint8_t)('0' + (n % 10));
        n /= 10;
    }
    // Reverse into `out`.
    for (int i = 0; i < len; i++) {
        out[i] = buf[len - 1 - i];
    }
    return len;
}

// ---- Main kernel -------------------------------------------------------

extern "C" __global__ void sha1_hasher(
    const uint8_t* __restrict__ pubkey,
    uint32_t pubkey_len,
    uint64_t start_counter,
    uint64_t n_per_thread,
    uint32_t current_best_level,
    unsigned long long* g_best_packed)
{
    const uint64_t tid = (uint64_t)blockIdx.x * blockDim.x + threadIdx.x;
    const uint64_t base = start_counter + tid * n_per_thread;

    // Local message buffer; reused per attempt.
    uint8_t msg[MAX_INPUT_BYTES];

    // Copy the pubkey prefix once — constant across the inner loop.
    for (uint32_t i = 0; i < pubkey_len; i++) {
        msg[i] = pubkey[i];
    }

    for (uint64_t i = 0; i < n_per_thread; i++) {
        const uint64_t counter = base + i;

        const int counter_len = utoa64_be(counter, msg + pubkey_len);
        const int total = (int)pubkey_len + counter_len;
        if (total + 1 + 8 > MAX_INPUT_BYTES) {
            // Should never happen for realistic pubkeys/counters; bail.
            return;
        }

        // SHA-1 padding.
        msg[total] = 0x80;
        const int blocks = (total + 1 + 8 + 63) / 64;
        const int padded = blocks * 64;
        for (int j = total + 1; j < padded - 8; j++) {
            msg[j] = 0;
        }
        // 64-bit BE length in bits.
        const uint64_t bit_len = (uint64_t)total * 8;
        msg[padded - 8] = (uint8_t)(bit_len >> 56);
        msg[padded - 7] = (uint8_t)(bit_len >> 48);
        msg[padded - 6] = (uint8_t)(bit_len >> 40);
        msg[padded - 5] = (uint8_t)(bit_len >> 32);
        msg[padded - 4] = (uint8_t)(bit_len >> 24);
        msg[padded - 3] = (uint8_t)(bit_len >> 16);
        msg[padded - 2] = (uint8_t)(bit_len >>  8);
        msg[padded - 1] = (uint8_t)(bit_len      );

        uint32_t h[5] = {
            0x67452301u, 0xEFCDAB89u, 0x98BADCFEu, 0x10325476u, 0xC3D2E1F0u,
        };
        for (int b = 0; b < blocks; b++) {
            sha1_block(h, msg + b * 64);
        }

        const uint32_t level = digest_level(h);
        if (level > current_best_level) {
            unsigned long long packed =
                ((unsigned long long)level << 56) | (unsigned long long)counter;
            atomicMax(g_best_packed, packed);
        }
    }
}
