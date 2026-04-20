// app/api/early-access/route.ts
import { NextRequest, NextResponse } from 'next/server';
import { Resend } from 'resend';

export async function POST(request: NextRequest) {
  try {
    const body = await request.json();
    const { name, email, company, stack, message } = body;

    if (!name || !email || !stack) {
      return NextResponse.json(
        { error: 'Name, email, and stack are required' },
        { status: 400 }
      );
    }

    // ←←← KEY CHANGE: Create Resend only when the request actually comes in
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
        <p style="font-size: 12px; color: #777;">Sent from datacontractgate.com marketing site • ${new Date().toISOString()}</p>
      `,
    });

    return NextResponse.json({ success: true });
  } catch (error) {
    console.error('Early access error:', error);
    return NextResponse.json(
      { error: 'Failed to send request' },
      { status: 500 }
    );
  }
}