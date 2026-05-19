import Link from "next/link";

export const metadata = { title: "Terms of Service — ContractGate" };

export default function TermsPage() {
  return (
    <div className="min-h-screen bg-[#0a0d12] flex flex-col items-center justify-center px-4 py-24">
      <div className="max-w-2xl w-full">
        <h1 className="text-2xl font-bold text-slate-100 mb-2">Terms of Service</h1>
        <p className="text-slate-500 text-sm mb-10">Last updated: May 2026</p>

        <div className="space-y-6 text-sm text-slate-400 leading-relaxed">
          <p>
            By accessing or using ContractGate (&ldquo;the Service&rdquo;), you agree to be
            bound by these Terms of Service. Please read them carefully.
          </p>

          <section>
            <h2 className="text-slate-200 font-semibold mb-2">1. Use of Service</h2>
            <p>
              You may use the Service only for lawful purposes and in accordance with
              these Terms. You are responsible for maintaining the security of your
              account credentials and for all activity that occurs under your account.
            </p>
          </section>

          <section>
            <h2 className="text-slate-200 font-semibold mb-2">2. Intellectual Property</h2>
            <p>
              The Service and its original content, features, and functionality are
              owned by ContractGate and are protected by applicable intellectual
              property laws. The open-source components of ContractGate are licensed
              under their respective open-source licenses.
            </p>
          </section>

          <section>
            <h2 className="text-slate-200 font-semibold mb-2">3. Data &amp; Privacy</h2>
            <p>
              Your use of the Service is also governed by our{" "}
              <Link href="/privacy" className="text-green-400 hover:text-green-300">
                Privacy Policy
              </Link>
              . By using the Service, you consent to the collection and use of
              information as described therein.
            </p>
          </section>

          <section>
            <h2 className="text-slate-200 font-semibold mb-2">4. Limitation of Liability</h2>
            <p>
              To the maximum extent permitted by law, ContractGate shall not be liable
              for any indirect, incidental, special, consequential, or punitive damages
              arising out of or relating to your use of the Service.
            </p>
          </section>

          <section>
            <h2 className="text-slate-200 font-semibold mb-2">5. Changes</h2>
            <p>
              We reserve the right to modify these Terms at any time. We will provide
              notice of significant changes by updating the date at the top of this
              page. Your continued use of the Service after changes constitutes
              acceptance of the new Terms.
            </p>
          </section>

          <section>
            <h2 className="text-slate-200 font-semibold mb-2">6. Contact</h2>
            <p>
              Questions about these Terms? Email us at{" "}
              <a
                href="mailto:sales@contractgate.io"
                className="text-green-400 hover:text-green-300"
              >
                sales@contractgate.io
              </a>
              .
            </p>
          </section>
        </div>

        <div className="mt-12 pt-6 border-t border-[#1f2937]">
          <Link href="/" className="text-sm text-slate-500 hover:text-slate-400 transition-colors">
            ← Back to ContractGate
          </Link>
        </div>
      </div>
    </div>
  );
}
