// app/api/early-access/route.ts
import { NextRequest, NextResponse } from 'next/server';
import { Resend } from 'resend';

export async function POST(request: NextRequest) {
  // CORS headers for cross-subdomain calls (www. → app.)
  const responseHeaders = new Headers();
  responseHeaders.set('Access-Control-Allow-Origin', 'https://www.datacontractgate.com');
  responseHeaders.set('Access-Control-Allow-Methods', 'POST, OPTIONS');
  responseHeaders.set('Access-Control-Allow-Headers', 'Content-Type');

  // Handle preflight OPTIONS request
  if (request.method === 'OPTIONS') {
    return NextResponse.json({}, { headers: responseHeaders });
  }

  try {
    const body = await request.json();
    const { name, email, company, stack, message } = body;

    if (!name || !email || !stack) {
      return NextResponse.json(
        { error: 'Name, email, and stack are required' },
        { status: 400, headers: responseHeaders }
      );
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
        <p style="font-size: 12px; color: #666;">Sent from datacontractgate.com • ${new Date().toISOString()}</p>
      `,
    });

    return NextResponse.json({ success: true }, { headers: responseHeaders });
  } catch (error: unknown) {
    console.error('Early access error:', error);
    return NextResponse.json(
      { error: 'Failed to send request' },
      { status: 500, headers: responseHeaders }
    );
  }
}