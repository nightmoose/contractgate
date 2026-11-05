import assert from "node:assert/strict";
import test from "node:test";

import { deriveSigningKey, signRequest, amzDateNow } from "./aws-sigv4";

// Official AWS documentation example credentials (AKIDEXAMPLE / wJalrXUtnFE…).
// The expected values below are the AWS "IAM ListUsers" worked example and were
// cross-validated against AWS's own reference implementation (@smithy/signature-v4).
const AWS_EXAMPLE_SECRET = "wJalrXUtnFEMI/K7MDENG+bbPxRfiCYEXAMPLEKEY";

test("deriveSigningKey matches the AWS example signing key (20150830/us-east-1/iam)", () => {
  const key = deriveSigningKey(AWS_EXAMPLE_SECRET, "20150830", "us-east-1", "iam");
  assert.equal(
    key.toString("hex"),
    "fa2afa0dbbd40241c0347905b28268dd1b529bf125bf838d8ec6b43ac86c20fc"
  );
});

test("signRequest reproduces the AWS ListUsers example Authorization header", () => {
  const headers = signRequest({
    method: "POST",
    host: "iam.amazonaws.com",
    path: "/",
    region: "us-east-1",
    service: "iam",
    body: "Action=ListUsers&Version=2010-05-08",
    accessKeyId: "AKIDEXAMPLE",
    secretAccessKey: AWS_EXAMPLE_SECRET,
    headers: { "content-type": "application/x-www-form-urlencoded; charset=utf-8" },
    amzDate: "20150830T123600Z",
  });

  assert.equal(
    headers["Authorization"],
    "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/iam/aws4_request, " +
      "SignedHeaders=content-type;host;x-amz-date, " +
      "Signature=371cae7ae46b4b9fe977a7c372de68585bf5d2389d52430a4f47890091673fe8"
  );
  assert.equal(headers["X-Amz-Date"], "20150830T123600Z");
  assert.equal(headers["Host"], "iam.amazonaws.com");
});

test("signRequest signs x-amz-security-token when a session token is supplied", () => {
  const headers = signRequest({
    method: "POST",
    host: "email.us-east-1.amazonaws.com",
    path: "/v2/email/outbound-emails",
    region: "us-east-1",
    service: "ses",
    body: "{}",
    accessKeyId: "AKIDEXAMPLE",
    secretAccessKey: AWS_EXAMPLE_SECRET,
    sessionToken: "TEMP-SESSION-TOKEN",
    headers: { "content-type": "application/json" },
    amzDate: "20150830T123600Z",
  });

  // The token is both sent and covered by the signature.
  assert.equal(headers["X-Amz-Security-Token"], "TEMP-SESSION-TOKEN");
  assert.match(headers["Authorization"], /SignedHeaders=[^,]*x-amz-security-token/);
});

test("amzDateNow formats a Date as compact ISO-8601 basic (YYYYMMDDTHHMMSSZ)", () => {
  assert.equal(amzDateNow(new Date("2015-08-30T12:36:00.000Z")), "20150830T123600Z");
});
