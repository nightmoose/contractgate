// app/api/early-access/route.ts
import { NextResponse } from "next/server";
import { SESv2Client, SendEmailCommand } from "@aws-sdk/client-sesv2";

// Standard AWS env vars (AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY) are picked up
// by the SDK's default credential provider chain — never hardcode credentials.
const AWS_REGION = process.env.AWS_REGION || "us-east-1";

// Verified sender identity on datacontractgate.com (see docs/email-sending-setup.md).
const FROM_ADDRESS = "DataContractGate <no-reply@datacontractgate.com>";

// Internal inbox that receives early-access requests. Kept out of source so the
// public address can change without a code deploy.
const NOTIFY_TO = process.env.EARLY_ACCESS_NOTIFY_TO;

if (!NOTIFY_TO) {
  console.error("[early-access] ❌ EARLY_ACCESS_NOTIFY_TO is not set!");
}

const CORS_HEADERS = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Methods": "POST, OPTIONS",
  "Access-Control-Allow-Headers": "Content-Type",
};

// Handle CORS preflight
export async function OPTIONS() {
  return new NextResponse(null, { status: 204, headers: CORS_HEADERS });
}

export async function POST(req: Request) {
  try {
    const { name, email, company, stack, message } = await req.json();

    if (!name || !email || !stack) {
      return NextResponse.json(
        { error: "Name, email, and stack are required." },
        { status: 400, headers: CORS_HEADERS }
      );
    }

    if (!NOTIFY_TO) {
      console.error(
        "[early-access] EARLY_ACCESS_NOTIFY_TO is not configured — cannot send notification."
      );
      return NextResponse.json(
        { error: "Something went wrong." },
        { status: 500, headers: CORS_HEADERS }
      );
    }

    const ses = new SESv2Client({ region: AWS_REGION });

    await ses.send(
      new SendEmailCommand({
        FromEmailAddress: FROM_ADDRESS,
        Destination: { ToAddresses: [NOTIFY_TO] },
        ReplyToAddresses: [email],
        Content: {
          Simple: {
            Subject: { Data: `Early Access Request from ${name}` },
            Body: {
              Html: {
                Data: `
        <h2>New Early Access Request</h2>
        <p><strong>Name:</strong> ${name}</p>
        <p><strong>Email:</strong> ${email}</p>
        <p><strong>Company / Role:</strong> ${company || '—'}</p>
        <p><strong>Stack:</strong> ${stack}</p>
        <p><strong>Message:</strong></p>
        <p>${message || 'No additional message provided.'}</p>
        <hr>
        <p style="font-size: 12px; color: #666;">Sent from datacontractgate.com marketing site • ${new Date().toISOString()}</p>
      `,
              },
            },
          },
        },
      })
    );

    return NextResponse.json({ success: true }, { headers: CORS_HEADERS });
  } catch (err) {
    console.error("[early-access]", err);
    return NextResponse.json(
      { error: "Something went wrong." },
      { status: 500, headers: CORS_HEADERS }
    );
  }
}
