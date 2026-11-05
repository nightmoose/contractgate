// Minimal AWS Signature Version 4 signer built on node:crypto.
//
// Just enough of the spec to sign a single JSON POST to an AWS REST/JSON API
// (SES v2 `SendEmail`) without pulling in the AWS SDK. Supports temporary
// credentials via `sessionToken` (X-Amz-Security-Token).
//
// Reference: https://docs.aws.amazon.com/general/latest/gr/sigv4-create-canonical-request.html
import crypto from "node:crypto";

export interface SignRequestInput {
  method: string;
  /** Host header, e.g. `email.us-east-1.amazonaws.com`. */
  host: string;
  /** Canonical (already URL-encoded) path, e.g. `/v2/email/outbound-emails`. */
  path: string;
  region: string;
  /** Signing service name, e.g. `ses`. */
  service: string;
  /** Raw request body (empty string for a bodyless request). */
  body: string;
  accessKeyId: string;
  secretAccessKey: string;
  /** Temporary-credential session token, signed as X-Amz-Security-Token. */
  sessionToken?: string;
  /** Extra headers to send and sign (e.g. `content-type`). Names are lowercased. */
  headers?: Record<string, string>;
  /**
   * ISO-basic timestamp `YYYYMMDDTHHMMSSZ`. Injectable so the signature is
   * deterministic in tests; defaults to the current UTC time.
   */
  amzDate?: string;
}

const sha256Hex = (data: string): string =>
  crypto.createHash("sha256").update(data, "utf8").digest("hex");

const hmac = (key: crypto.BinaryLike | Buffer, data: string): Buffer =>
  crypto.createHmac("sha256", key).update(data, "utf8").digest();

/** Current UTC time formatted as `YYYYMMDDTHHMMSSZ`. */
export function amzDateNow(date: Date = new Date()): string {
  return date.toISOString().replace(/[:-]|\.\d{3}/g, "");
}

/** Derive the SigV4 signing key: HMAC chain over date → region → service. */
export function deriveSigningKey(
  secretAccessKey: string,
  dateStamp: string,
  region: string,
  service: string
): Buffer {
  const kDate = hmac(`AWS4${secretAccessKey}`, dateStamp);
  const kRegion = hmac(kDate, region);
  const kService = hmac(kRegion, service);
  return hmac(kService, "aws4_request");
}

/**
 * Sign the request and return the full set of headers to send (including
 * `Authorization`, `Host`, `X-Amz-Date`, and any session token).
 */
export function signRequest(input: SignRequestInput): Record<string, string> {
  const amzDate = input.amzDate ?? amzDateNow();
  const dateStamp = amzDate.slice(0, 8);

  // Collect headers to sign: host + x-amz-date are mandatory; merge caller
  // extras and the optional security token. Keys are lowercased.
  const signedHeaderMap: Record<string, string> = {
    host: input.host,
    "x-amz-date": amzDate,
  };
  for (const [name, value] of Object.entries(input.headers ?? {})) {
    signedHeaderMap[name.toLowerCase()] = value;
  }
  if (input.sessionToken) {
    signedHeaderMap["x-amz-security-token"] = input.sessionToken;
  }

  const sortedNames = Object.keys(signedHeaderMap).sort();
  const canonicalHeaders = sortedNames
    .map((name) => `${name}:${signedHeaderMap[name].trim().replace(/\s+/g, " ")}\n`)
    .join("");
  const signedHeaders = sortedNames.join(";");

  const payloadHash = sha256Hex(input.body);
  const canonicalRequest = [
    input.method,
    input.path,
    "", // canonical query string — none for our requests
    canonicalHeaders,
    signedHeaders,
    payloadHash,
  ].join("\n");

  const scope = `${dateStamp}/${input.region}/${input.service}/aws4_request`;
  const stringToSign = [
    "AWS4-HMAC-SHA256",
    amzDate,
    scope,
    sha256Hex(canonicalRequest),
  ].join("\n");

  const signingKey = deriveSigningKey(
    input.secretAccessKey,
    dateStamp,
    input.region,
    input.service
  );
  const signature = crypto
    .createHmac("sha256", signingKey)
    .update(stringToSign, "utf8")
    .digest("hex");

  const authorization =
    `AWS4-HMAC-SHA256 Credential=${input.accessKeyId}/${scope}, ` +
    `SignedHeaders=${signedHeaders}, Signature=${signature}`;

  // Return headers with their canonical casing for the actual HTTP request.
  const outHeaders: Record<string, string> = {
    Host: input.host,
    "X-Amz-Date": amzDate,
    Authorization: authorization,
  };
  for (const [name, value] of Object.entries(input.headers ?? {})) {
    outHeaders[name] = value;
  }
  if (input.sessionToken) {
    outHeaders["X-Amz-Security-Token"] = input.sessionToken;
  }
  return outHeaders;
}
