// First boot writes unique self-host secrets. A later boot keeps the file.
// `node ops/selfhost/ensure-secrets.mjs --print` prints a fresh set and
// does not write a file.
import { createHmac, randomBytes } from "node:crypto";
import { existsSync, mkdirSync, renameSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";

// Public values that used to ship in the repo. Keep in sync with guard.sh.
const PLACEHOLDER_JWT = "selfhost-placeholder-jwt-secret-change-me";
const PLACEHOLDER_ANON =
  "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJyb2xlIjoiYW5vbiIsImlzcyI6InN1cGFiYXNlLWRlbW8iLCJpYXQiOjE3MDAwMDAwMDAsImV4cCI6MjAwMDAwMDAwMH0.FQCUyi7iBQeSGaeFYGpBRscSATPfYZlZUEi6YhcvRgo";
const PLACEHOLDER_SERVICE =
  "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJyb2xlIjoic2VydmljZV9yb2xlIiwiaXNzIjoic3VwYWJhc2UtZGVtbyIsImlhdCI6MTcwMDAwMDAwMCwiZXhwIjoyMDAwMDAwMDAwfQ.fVtRW0gwj7WJkhMjWIaC40ZZOS9PaN4DMe_DNtFvIJM";

const REFUSE =
  "self-host: refusing to start. Ports bind to a non-local address, but the JWT secret or API keys are public placeholders or empty. Use the default bind 127.0.0.1, or generate unique keys (docs/self-host-reference.md) before exposing the stack.";

function b64url(input) {
  return Buffer.from(input).toString("base64url");
}

function sign(payload, secret) {
  const header = b64url(JSON.stringify({ alg: "HS256", typ: "JWT" }));
  const body = b64url(JSON.stringify(payload));
  const data = `${header}.${body}`;
  const sig = createHmac("sha256", secret).update(data).digest("base64url");
  return `${data}.${sig}`;
}

function isLocal(bind) {
  return bind === "127.0.0.1" || bind === "localhost" || bind === "::1";
}

function isPlaceholder(secrets) {
  return (
    !secrets.SUPABASE_JWT_SECRET ||
    !secrets.SUPABASE_ANON_KEY ||
    !secrets.SUPABASE_SERVICE_ROLE_KEY ||
    !secrets.SELFHOST_DB_PASSWORD ||
    secrets.SUPABASE_JWT_SECRET === PLACEHOLDER_JWT ||
    secrets.SUPABASE_ANON_KEY === PLACEHOLDER_ANON ||
    secrets.SUPABASE_SERVICE_ROLE_KEY === PLACEHOLDER_SERVICE
  );
}

// Values are sourced by /bin/sh. Reject quotes and shell expansions.
function envSafe(value) {
  return value.length > 0 && /^[\x21-\x7E]+$/.test(value) && !/['"\\$`]/.test(value);
}

function generate() {
  const secret = randomBytes(48).toString("base64url");
  const iat = Math.floor(Date.now() / 1000);
  const exp = iat + 60 * 60 * 24 * 365 * 10;
  const claims = (role) => ({ role, iss: "contractgate-selfhost", iat, exp });
  return {
    SUPABASE_JWT_SECRET: secret,
    SUPABASE_ANON_KEY: sign(claims("anon"), secret),
    SUPABASE_SERVICE_ROLE_KEY: sign(claims("service_role"), secret),
    SELFHOST_DB_PASSWORD: randomBytes(24).toString("hex"),
  };
}

function writeEnv(file, secrets) {
  const body = Object.entries(secrets)
    .map(([key, value]) => `${key}=${value}\n`)
    .join("");
  mkdirSync(dirname(file), { recursive: true });
  const tmp = `${file}.tmp`;
  writeFileSync(tmp, body, { mode: 0o644 });
  renameSync(tmp, file);
}

const printOnly = process.argv.includes("--print");
if (printOnly) {
  const secrets = generate();
  process.stdout.write(
    Object.entries(secrets)
      .map(([key, value]) => `${key}=${value}\n`)
      .join(""),
  );
  process.exit(0);
}

const file = process.env.SECRETS_FILE || "/selfhost/secrets/secrets.env";
if (existsSync(file)) {
  process.exit(0);
}

const supplied = {
  SUPABASE_JWT_SECRET: (process.env.SUPABASE_JWT_SECRET || "").trim(),
  SUPABASE_ANON_KEY: (process.env.SUPABASE_ANON_KEY || "").trim(),
  SUPABASE_SERVICE_ROLE_KEY: (process.env.SUPABASE_SERVICE_ROLE_KEY || "").trim(),
  SELFHOST_DB_PASSWORD: (process.env.SELFHOST_DB_PASSWORD || "").trim(),
};
const present = Object.values(supplied).filter(Boolean).length;
let secrets;
if (present === 0) {
  secrets = generate();
} else if (present === 4) {
  if (supplied.SUPABASE_JWT_SECRET.length < 32) {
    console.error("self-host: SUPABASE_JWT_SECRET must be at least 32 characters.");
    process.exit(1);
  }
  for (const [key, value] of Object.entries(supplied)) {
    if (!envSafe(value)) {
      console.error(
        `self-host: ${key} has characters that cannot be stored in the secrets file.`,
      );
      process.exit(1);
    }
  }
  secrets = supplied;
} else {
  console.error(
    "self-host: set SUPABASE_JWT_SECRET, SUPABASE_ANON_KEY, SUPABASE_SERVICE_ROLE_KEY, and SELFHOST_DB_PASSWORD together, or leave all four unset to generate them.",
  );
  process.exit(1);
}

const bind = (process.env.SELFHOST_BIND || "127.0.0.1").trim() || "127.0.0.1";
if (!isLocal(bind) && isPlaceholder(secrets)) {
  console.error(REFUSE);
  process.exit(1);
}

writeEnv(file, secrets);
