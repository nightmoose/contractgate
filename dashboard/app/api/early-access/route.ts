// app/api/early-access/route.ts
import { NextResponse } from "next/server";
import { Resend } from 'resend';

if (!process.env.RESEND_API_KEY) {
  console.error("[early-access] ❌ RESEND_API_KEY is not set!");
}

export async function POST(req: Request) {
  try {
    const { name, email, company, stack, message } = await req.json();

    if (!name || !email || !stack) {
      return NextResponse.json({ error: "Name, email, and stack are required." }, { status: 400 });
    }

    const resend = new Resend(process.env.RESEND_API_KEY);

    await resend.emails.send({
      from: 'ContractGate Early Access <datacontractgate@nightmoose.com>',
      to: 'datacontractgate_signup@nightmoose.com',

      replyTo: email,
      subject: `Early Access Request from ${name}`,
      html: `
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
    });

    return NextResponse.json({ success: true });
  } catch (err) {
    console.error("[early-access]", err);
    return NextResponse.json({ error: "Something went wrong." }, { status: 500 });
  }
}