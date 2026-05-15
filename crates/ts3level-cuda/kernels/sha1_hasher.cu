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

// With host-side midstate precompute, the kernel only processes the
// pubkey tail (= bytes after the host-folded prefix) plus the counter
// ASCII and SHA-1 padding/length. For the typical TS3 P-256 base64
// pubkey of ~108 chars the prefix takes one 64-byte block and the tail
// fits well inside two SHA-1 blocks. Cap at two blocks (128 bytes) so
// the per-thread stack frame shrinks accordingly.
#define MAX_INPUT_BYTES   128
#define MAX_INPUT_BLOCKS  2      // 128 / 64

// ---- SHA-1 core --------------------------------------------------------
//
// The round-function macros and the SHA1_STEP form are derived from
// hashcat's `OpenCL/inc_hash_sha1.h` (MIT-licensed). The textbook
// expressions for Ch and Maj have been rewritten to the
// (z ^ (x & (y ^ z))) and ((x & y) | (z & (x ^ y))) forms — both are
// 3-input boolean functions of one 32-bit output, which nvcc auto-lowers
// to a single `LOP3.LUT` instruction on sm_50+. 32-bit rotates go
// through `__funnelshift_l`, a single SASS instruction on sm_70+ and a
// short emulation on older arches.

#define SHA1_F0(b, c, d)  ((d) ^ ((b) & ((c) ^ (d))))            // Ch
#define SHA1_F1(b, c, d)  ((b) ^ (c) ^ (d))                       // Parity
#define SHA1_F2(b, c, d)  (((b) & (c)) | ((d) & ((b) ^ (c))))     // Maj

#define SHA1_K0 0x5A827999u
#define SHA1_K1 0x6ED9EBA1u
#define SHA1_K2 0x8F1BBCDCu
#define SHA1_K3 0xCA62C1D6u

#define SHA1_STEP(f, a, b, c, d, e, w_t, K) do {       \
    (e) += (K);                                         \
    (e) += (w_t) + f((b), (c), (d));                    \
    (e) += __funnelshift_l((a), (a), 5);                \
    (b)  = __funnelshift_l((b), (b), 30);               \
} while (0)

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
        const uint32_t x = w[i-3] ^ w[i-8] ^ w[i-14] ^ w[i-16];
        w[i] = __funnelshift_l(x, x, 1);
    }

    uint32_t a = h[0], b = h[1], c = h[2], d = h[3], e = h[4];

    // Round 1: f = Ch, K = 0x5A827999. After each step the registers
    // rotate (e,a,b,c,d) ← (d,e,a,rotl(b,30),c) which is the standard
    // SHA-1 schedule. We unroll the rotation explicitly into five
    // SHA1_STEP invocations with cycled operand order, identical to
    // hashcat's expansion.
#pragma unroll
    for (int i = 0; i < 20; i += 5) {
        SHA1_STEP(SHA1_F0, a, b, c, d, e, w[i + 0], SHA1_K0);
        SHA1_STEP(SHA1_F0, e, a, b, c, d, w[i + 1], SHA1_K0);
        SHA1_STEP(SHA1_F0, d, e, a, b, c, w[i + 2], SHA1_K0);
        SHA1_STEP(SHA1_F0, c, d, e, a, b, w[i + 3], SHA1_K0);
        SHA1_STEP(SHA1_F0, b, c, d, e, a, w[i + 4], SHA1_K0);
    }
#pragma unroll
    for (int i = 20; i < 40; i += 5) {
        SHA1_STEP(SHA1_F1, a, b, c, d, e, w[i + 0], SHA1_K1);
        SHA1_STEP(SHA1_F1, e, a, b, c, d, w[i + 1], SHA1_K1);
        SHA1_STEP(SHA1_F1, d, e, a, b, c, w[i + 2], SHA1_K1);
        SHA1_STEP(SHA1_F1, c, d, e, a, b, w[i + 3], SHA1_K1);
        SHA1_STEP(SHA1_F1, b, c, d, e, a, w[i + 4], SHA1_K1);
    }
#pragma unroll
    for (int i = 40; i < 60; i += 5) {
        SHA1_STEP(SHA1_F2, a, b, c, d, e, w[i + 0], SHA1_K2);
        SHA1_STEP(SHA1_F2, e, a, b, c, d, w[i + 1], SHA1_K2);
        SHA1_STEP(SHA1_F2, d, e, a, b, c, w[i + 2], SHA1_K2);
        SHA1_STEP(SHA1_F2, c, d, e, a, b, w[i + 3], SHA1_K2);
        SHA1_STEP(SHA1_F2, b, c, d, e, a, w[i + 4], SHA1_K2);
    }
#pragma unroll
    for (int i = 60; i < 80; i += 5) {
        SHA1_STEP(SHA1_F1, a, b, c, d, e, w[i + 0], SHA1_K3);
        SHA1_STEP(SHA1_F1, e, a, b, c, d, w[i + 1], SHA1_K3);
        SHA1_STEP(SHA1_F1, d, e, a, b, c, w[i + 2], SHA1_K3);
        SHA1_STEP(SHA1_F1, c, d, e, a, b, w[i + 3], SHA1_K3);
        SHA1_STEP(SHA1_F1, b, c, d, e, a, w[i + 4], SHA1_K3);
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

// Count the decimal digits of `n` (returns 1..=20). Used to seed the
// per-thread counter ASCII state; subsequent iterations only run this
// when the digit count changes (crossing a 10^k boundary).
__device__ __forceinline__ int decimal_digit_count(uint64_t n) {
    int len = 1;
    while (n >= 10) {
        n /= 10;
        len++;
    }
    return len;
}

// Encode `n` as exactly `len` decimal ASCII characters at `out[0..len]`,
// most-significant digit first. Caller guarantees `len == decimal_digit_count(n)`.
__device__ __forceinline__ void encode_decimal(uint64_t n, int len, uint8_t* out) {
    for (int j = len - 1; j >= 0; j--) {
        out[j] = (uint8_t)('0' + (n % 10));
        n /= 10;
    }
}

// ---- Main kernel -------------------------------------------------------
//
// Reporting uses two separate device slots, not one packed `(level <<
// 56) | counter` word. Packing breaks once `counter >= 2^56` (which is
// reached around level 57), because the counter's high bits then bleed
// into the level field. With two slots there is still a benign race
// between threads writing concurrently — `g_best_counter` may briefly
// not match `g_best_level` — but the host always CPU-verifies the
// counter's level before persisting, so the worst case is "wait for the
// next batch".
//
// The host precomputes the SHA-1 state of the constant pubkey prefix
// (full 64-byte blocks of pubkey bytes) and passes it as `midstate`
// plus `prefix_bit_len`. The kernel then only processes the tail
// (pubkey remainder + counter ASCII + padding) per iteration. For
// the typical 108-char TS3 pubkey this drops one full SHA-1 block out
// of three per attempt.

extern "C" __global__ void sha1_hasher(const uint8_t* __restrict__ pubkey_tail,
                                       uint32_t pubkey_tail_len,
                                       uint64_t prefix_bit_len,
                                       const uint32_t* __restrict__ midstate,
                                       uint64_t start_counter,
                                       uint64_t n_per_thread,
                                       uint32_t current_best_level,
                                       unsigned int* g_best_level,
                                       unsigned long long* g_best_counter)
{
    // Defense-in-depth bound check. With midstate, msg[] only holds the
    // tail bytes — pubkey remainder + up to 20 counter digits + 9 bytes
    // of SHA-1 padding/length. The host enforces this upstream.
    if (pubkey_tail_len + /*max counter*/ 20 + /*0x80*/ 1 + /*length*/ 8 > MAX_INPUT_BYTES) {
        return;
    }

    const uint64_t tid = (uint64_t)blockIdx.x * blockDim.x + threadIdx.x;
    const uint64_t base = start_counter + tid * n_per_thread;

    // Cache the midstate in registers — read once per thread instead of
    // per iteration. msg[] still lives in thread-local memory; that is
    // the next optimisation tranche (Phase C-2b).
    const uint32_t h0_init = midstate[0];
    const uint32_t h1_init = midstate[1];
    const uint32_t h2_init = midstate[2];
    const uint32_t h3_init = midstate[3];
    const uint32_t h4_init = midstate[4];

    uint8_t msg[MAX_INPUT_BYTES];

    // Copy the pubkey tail (constant across the inner loop).
    for (uint32_t i = 0; i < pubkey_tail_len; i++) {
        msg[i] = pubkey_tail[i];
    }

    // Build msg[] once for the starting counter: counter digits, then
    // SHA-1 0x80 marker, zero-fill, and 8-byte length suffix. The inner
    // loop bumps the counter digits in place and only re-bakes the
    // tail when the digit count changes (i.e. when counter crosses a
    // 10^k boundary, which happens at most log10(n_per_thread) times
    // across the whole launch).
    uint64_t counter = base;
    int counter_len = decimal_digit_count(counter);
    int total = (int)pubkey_tail_len + counter_len;
    int blocks = (total + 1 + 8 + 63) / 64;
    int padded = blocks * 64;
    encode_decimal(counter, counter_len, msg + pubkey_tail_len);
    msg[total] = 0x80;
    for (int j = total + 1; j < padded - 8; j++) {
        msg[j] = 0;
    }
    {
        const uint64_t bit_len = prefix_bit_len + (uint64_t)total * 8;
        msg[padded - 8] = (uint8_t)(bit_len >> 56);
        msg[padded - 7] = (uint8_t)(bit_len >> 48);
        msg[padded - 6] = (uint8_t)(bit_len >> 40);
        msg[padded - 5] = (uint8_t)(bit_len >> 32);
        msg[padded - 4] = (uint8_t)(bit_len >> 24);
        msg[padded - 3] = (uint8_t)(bit_len >> 16);
        msg[padded - 2] = (uint8_t)(bit_len >> 8);
        msg[padded - 1] = (uint8_t)(bit_len);
    }

    for (uint64_t i = 0; i < n_per_thread; i++) {
        uint32_t h[5] = { h0_init, h1_init, h2_init, h3_init, h4_init };
        for (int b = 0; b < blocks; b++) {
            sha1_block(h, msg + b * 64);
        }

        const uint32_t level = digest_level(h);
        if (level > current_best_level) {
            // Two-stage filter: the kernel-argument `current_best_level`
            // is captured at launch time. As the run progresses other
            // threads in this grid may have already pushed
            // `g_best_level` much higher; a plain volatile read is good
            // enough to filter the bulk of late-arriving matches before
            // they reach the serialising atomicMax. Without this, every
            // level≥1 hit (~50 % of iterations on a from-scratch run)
            // serialised on a single device slot and the kernel was
            // atomic-bound rather than compute-bound.
            const uint32_t observed_best = *(volatile unsigned int*)g_best_level;
            if (level > observed_best) {
                unsigned int prev_level = atomicMax(g_best_level, level);
                if (level > prev_level) {
                    atomicExch(g_best_counter, (unsigned long long)counter);
                }
            }
        }

        // Advance to the next counter. Skip on the last iteration so
        // we don't pay for the bump when the result is discarded.
        if (i + 1 >= n_per_thread) {
            break;
        }
        counter++;
        // In-place increment of the trailing digits. The loop falls
        // through to msg[p]++ in the common no-carry case (~90 % of
        // iterations for decimal counters); a chain of '9' → '0' walks
        // back across consecutive 9s; and if we walk past the most
        // significant digit, the counter just gained a digit and the
        // padding/length suffix shift back by one byte, so re-bake the
        // tail of msg[].
        int p = (int)pubkey_tail_len + counter_len - 1;
        while (p >= (int)pubkey_tail_len && msg[p] == '9') {
            msg[p] = '0';
            p--;
        }
        if (p < (int)pubkey_tail_len) {
            counter_len++;
            total = (int)pubkey_tail_len + counter_len;
            const int new_blocks = (total + 1 + 8 + 63) / 64;
            const int new_padded = new_blocks * 64;
            // Re-encode the entire counter into the (now shifted)
            // digit slot. Skip the leading '1' written below.
            encode_decimal(counter, counter_len, msg + pubkey_tail_len);
            msg[total] = 0x80;
            for (int j = total + 1; j < new_padded - 8; j++) {
                msg[j] = 0;
            }
            const uint64_t bit_len = prefix_bit_len + (uint64_t)total * 8;
            msg[new_padded - 8] = (uint8_t)(bit_len >> 56);
            msg[new_padded - 7] = (uint8_t)(bit_len >> 48);
            msg[new_padded - 6] = (uint8_t)(bit_len >> 40);
            msg[new_padded - 5] = (uint8_t)(bit_len >> 32);
            msg[new_padded - 4] = (uint8_t)(bit_len >> 24);
            msg[new_padded - 3] = (uint8_t)(bit_len >> 16);
            msg[new_padded - 2] = (uint8_t)(bit_len >> 8);
            msg[new_padded - 1] = (uint8_t)(bit_len);
            blocks = new_blocks;
            padded = new_padded;
        } else {
            msg[p]++;
        }
    }
}
