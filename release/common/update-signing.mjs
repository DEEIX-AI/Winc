// Sparkle's current exported key is a base64-encoded 32-byte Ed25519 seed.
// Node's crypto implementation keeps signing portable across all build runners.
import { createPrivateKey, createPublicKey, sign, verify } from "node:crypto";

export function signingKey(env = process.env) {
  const encoded = (env.WINC_UPDATE_PRIVATE_KEY ?? "").trim();
  if (!/^[A-Za-z0-9+/]{43}=$/.test(encoded)) {
    throw new Error("WINC_UPDATE_PRIVATE_KEY must be a base64-encoded 32-byte Sparkle seed");
  }
  const seed = Buffer.from(encoded, "base64");
  if (seed.length !== 32 || seed.toString("base64") !== encoded) throw new Error("Invalid Sparkle seed");
  return createPrivateKey({
    key: Buffer.concat([Buffer.from("302e020100300506032b657004220420", "hex"), seed]),
    format: "der", type: "pkcs8",
  });
}

export function publicKey(key) {
  return createPublicKey(key).export({ format: "der", type: "spki" }).subarray(-32).toString("base64");
}

export function verificationKey(encoded) {
  const bytes = Buffer.from(encoded, "base64");
  if (bytes.length !== 32) throw new Error("Invalid update public key");
  return createPublicKey({
    key: Buffer.concat([Buffer.from("302a300506032b6570032100", "hex"), bytes]),
    format: "der", type: "spki",
  });
}

export function signBytes(bytes, key) {
  const signature = sign(null, bytes, key);
  if (!verify(null, bytes, createPublicKey(key), signature)) throw new Error("Update signature verification failed");
  return signature.toString("base64");
}

export function signFeed(xml, key) {
  const bytes = Buffer.from(xml, "utf8");
  return `${xml}<!-- sparkle-signatures:\nedSignature: ${signBytes(bytes, key)}\nlength: ${bytes.length}\n-->\n`;
}

export function verifyFeed(feed, key) {
  const bytes = Buffer.from(feed);
  const at = bytes.lastIndexOf("<!-- sparkle-signatures:");
  const block = bytes.subarray(at).toString("utf8");
  const length = Number(/^length: (\d+)$/m.exec(block)?.[1]);
  const signature = /^edSignature: ([A-Za-z0-9+/=]+)$/m.exec(block)?.[1];
  return at >= 0 && Number.isSafeInteger(length) && length >= 0 && length <= at && !!signature
    && verify(null, bytes.subarray(0, length), key, Buffer.from(signature, "base64"));
}
