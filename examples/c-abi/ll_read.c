/* Minimal, dependency-free read call against a LedgerLens score contract
 * (issue #1438 spike). Uses only libc and the generated ABI reference
 * `schemas/c/ledgerlens_score.h` — no Soroban SDK, no XDR library.
 *
 *   ll_read encode <contract C...> <function> [sym:X | u32:N | addr:G...]...
 *       prints a base64 TransactionEnvelope for RPC `simulateTransaction`
 *   ll_read decode <base64 ScVal>
 *       decodes a `u32` / `bool` / `void` return value
 *
 * `read.sh` wires both halves to curl. Build: cc -std=c99 -I../../schemas/c ll_read.c
 */
#include "ledgerlens_score.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* XDR constants from the Stellar protocol (not contract-specific). */
#define ENVELOPE_TYPE_TX 2
#define KEY_TYPE_ED25519 0
#define OP_INVOKE_HOST_FUNCTION 24
#define HOST_FUNCTION_INVOKE_CONTRACT 0
#define SC_ADDRESS_ACCOUNT 0
#define SC_ADDRESS_CONTRACT 1

static uint8_t buf[4096];
static size_t len;

static void put_u32(uint32_t v) {
    buf[len++] = (uint8_t)(v >> 24);
    buf[len++] = (uint8_t)(v >> 16);
    buf[len++] = (uint8_t)(v >> 8);
    buf[len++] = (uint8_t)v;
}

static void put_bytes(const uint8_t *p, size_t n) {
    memcpy(buf + len, p, n);
    len += n;
    while (len % 4) buf[len++] = 0; /* XDR 4-byte padding */
}

static void put_string(const char *s) {
    put_u32((uint32_t)strlen(s));
    put_bytes((const uint8_t *)s, strlen(s));
}

static uint16_t crc16_xmodem(const uint8_t *p, size_t n) {
    uint16_t crc = 0;
    for (size_t i = 0; i < n; i++) {
        crc ^= (uint16_t)(p[i] << 8);
        for (int b = 0; b < 8; b++)
            crc = (uint16_t)((crc & 0x8000) ? (crc << 1) ^ 0x1021 : crc << 1);
    }
    return crc;
}

/* Decode a StrKey ("G..." / "C...") into its 32-byte payload. */
static int strkey_decode(const ll_address_t *a, uint8_t version, uint8_t out[32]) {
    static const char *alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    uint8_t raw[35];
    size_t n = 0;
    uint32_t acc = 0;
    int bits = 0;
    if (strlen(a->strkey) != 56) return -1;
    for (int i = 0; i < 56; i++) {
        const char *c = strchr(alphabet, a->strkey[i]);
        if (!c || !*c) return -1;
        acc = (acc << 5) | (uint32_t)(c - alphabet);
        bits += 5;
        if (bits >= 8) {
            bits -= 8;
            raw[n++] = (uint8_t)(acc >> bits);
        }
    }
    if (n != 35 || raw[0] != version) return -1;
    if (crc16_xmodem(raw, 33) != (uint16_t)(raw[33] | (raw[34] << 8))) return -1;
    memcpy(out, raw + 1, 32);
    return 0;
}

static int put_arg(const char *arg) {
    if (strncmp(arg, "sym:", 4) == 0) {
        put_u32(LL_SCV_SYMBOL);
        put_string(arg + 4);
    } else if (strncmp(arg, "u32:", 4) == 0) {
        put_u32(LL_SCV_U32);
        put_u32((uint32_t)strtoul(arg + 4, NULL, 10));
    } else if (strncmp(arg, "addr:", 5) == 0) {
        ll_address_t a;
        uint8_t key[32];
        snprintf(a.strkey, sizeof a.strkey, "%s", arg + 5);
        put_u32(LL_SCV_ADDRESS);
        if (a.strkey[0] == 'G' && strkey_decode(&a, 6 << 3, key) == 0) {
            put_u32(SC_ADDRESS_ACCOUNT);
            put_u32(KEY_TYPE_ED25519);
        } else if (a.strkey[0] == 'C' && strkey_decode(&a, 2 << 3, key) == 0) {
            put_u32(SC_ADDRESS_CONTRACT);
        } else {
            return -1;
        }
        put_bytes(key, 32);
    } else {
        return -1;
    }
    return 0;
}

static void base64_print(const uint8_t *p, size_t n) {
    static const char *t = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for (size_t i = 0; i < n; i += 3) {
        uint32_t v = (uint32_t)p[i] << 16 | (i + 1 < n ? p[i + 1] << 8 : 0) | (i + 2 < n ? p[i + 2] : 0);
        putchar(t[v >> 18 & 63]);
        putchar(t[v >> 12 & 63]);
        putchar(i + 1 < n ? t[v >> 6 & 63] : '=');
        putchar(i + 2 < n ? t[v & 63] : '=');
    }
    putchar('\n');
}

static size_t base64_decode(const char *s, uint8_t *out) {
    static const char *t = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    uint32_t acc = 0;
    int bits = 0;
    size_t n = 0;
    for (; *s && *s != '='; s++) {
        const char *c = strchr(t, *s);
        if (!c) continue;
        acc = (acc << 6) | (uint32_t)(c - t);
        bits += 6;
        if (bits >= 8) {
            bits -= 8;
            out[n++] = (uint8_t)(acc >> bits);
        }
    }
    return n;
}

static int encode(int argc, char **argv) {
    ll_address_t contract;
    uint8_t contract_id[32];
    const uint8_t source[32] = {0}; /* simulation needs no real source account */
    snprintf(contract.strkey, sizeof contract.strkey, "%s", argv[0]);
    if (strkey_decode(&contract, 2 << 3, contract_id) != 0) {
        fprintf(stderr, "invalid contract StrKey\n");
        return 1;
    }
    put_u32(ENVELOPE_TYPE_TX);
    put_u32(KEY_TYPE_ED25519); /* sourceAccount: MuxedAccount */
    put_bytes(source, 32);
    put_u32(100);              /* fee */
    put_u32(0); put_u32(0);    /* seqNum (int64) */
    put_u32(0);                /* cond: PRECOND_NONE */
    put_u32(0);                /* memo: MEMO_NONE */
    put_u32(1);                /* operations<1> */
    put_u32(0);                /* op sourceAccount: absent */
    put_u32(OP_INVOKE_HOST_FUNCTION);
    put_u32(HOST_FUNCTION_INVOKE_CONTRACT);
    put_u32(SC_ADDRESS_CONTRACT);
    put_bytes(contract_id, 32);
    put_string(argv[1]);       /* functionName: ScSymbol */
    put_u32((uint32_t)(argc - 2));
    for (int i = 2; i < argc; i++) {
        if (put_arg(argv[i]) != 0) {
            fprintf(stderr, "unsupported argument `%s`\n", argv[i]);
            return 1;
        }
    }
    put_u32(0);                /* auth<> */
    put_u32(0);                /* tx ext */
    put_u32(0);                /* signatures<> */
    base64_print(buf, len);
    return 0;
}

static int decode(const char *b64) {
    uint8_t v[256];
    size_t n = base64_decode(b64, v);
    uint32_t tag, word;
    if (n < 4) return 1;
    tag = (uint32_t)v[0] << 24 | v[1] << 16 | v[2] << 8 | v[3];
    word = n >= 8 ? (uint32_t)v[4] << 24 | v[5] << 16 | v[6] << 8 | v[7] : 0;
    switch (tag) {
    case LL_SCV_U32: printf("u32 %u\n", word); return 0;
    case LL_SCV_BOOL: printf("bool %s\n", word ? "true" : "false"); return 0;
    case LL_SCV_VOID: printf("void\n"); return 0;
    case LL_SCV_ERROR: printf("error (contract code %u)\n", n >= 12 ? (uint32_t)v[8] << 24 | v[9] << 16 | v[10] << 8 | v[11] : 0); return 0;
    default: printf("ScVal tag %u (%zu bytes) — decode per ledgerlens_score.h\n", tag, n); return 0;
    }
}

int main(int argc, char **argv) {
    if (argc >= 4 && strcmp(argv[1], "encode") == 0) return encode(argc - 2, argv + 2);
    if (argc == 3 && strcmp(argv[1], "decode") == 0) return decode(argv[2]);
    fprintf(stderr, "usage: %s encode <contract> <fn> [args...] | decode <scval-b64>\n", argv[0]);
    return 2;
}
