// Minimal, dependency-free BIP-39 mnemonic generator (English, 128-bit → 12
// words). Self-contained so the messaging e2e needs no npm install in this
// pnpm-enforced repo. Standard algorithm: 16 random entropy bytes, append the
// first ENT/32 = 4 bits of SHA-256(entropy) as checksum, slice into 11-bit
// groups, map each to the canonical English wordlist.
//
// We only need *fresh, valid* mnemonics per run so every test identity is a
// brand-new tiny.place cryptoId (avoids colliding with prekey state a previous
// run already published to the backend, which would 409 on re-provision).
import { createHash, randomBytes } from "node:crypto";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const HERE = dirname(fileURLToPath(import.meta.url));
const WORDLIST = readFileSync(join(HERE, "bip39-english.txt"), "utf8")
  .split("\n")
  .map((w) => w.trim())
  .filter(Boolean);

if (WORDLIST.length !== 2048) {
  throw new Error(`bip39 wordlist must have 2048 words, got ${WORDLIST.length}`);
}

/** Generate a fresh, checksum-valid 12-word English BIP-39 mnemonic. */
export function generateMnemonic() {
  const entropy = randomBytes(16); // 128 bits
  const checksum = createHash("sha256").update(entropy).digest();

  // Build a big-endian bit string of entropy (128) + checksum (4) = 132 bits.
  let bits = "";
  for (const byte of entropy) bits += byte.toString(2).padStart(8, "0");
  bits += checksum[0].toString(2).padStart(8, "0").slice(0, 4);

  const words = [];
  for (let i = 0; i < bits.length; i += 11) {
    words.push(WORDLIST[parseInt(bits.slice(i, i + 11), 2)]);
  }
  return words.join(" ");
}
