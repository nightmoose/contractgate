"use client";

import { forwardRef, useImperativeHandle, useRef } from "react";
import { Turnstile, type TurnstileInstance } from "@marsidev/react-turnstile";

// Supabase Auth verifies this token server-side (Auth → Bot and Abuse
// Protection) on signUp, signInWithPassword and resetPasswordForEmail.
//
// No fallback key, ever. When NEXT_PUBLIC_TURNSTILE_SITE_KEY is unset this
// renders nothing and forms submit without a token, which is correct only while
// Supabase CAPTCHA is off. Rollout order: set the site key in Vercel and
// redeploy first, then enable CAPTCHA in Supabase. See docs/auth-reference.md.
export const TURNSTILE_SITE_KEY = process.env.NEXT_PUBLIC_TURNSTILE_SITE_KEY || "";
export const captchaEnabled = TURNSTILE_SITE_KEY !== "";

export interface AuthCaptchaHandle {
  /** Tokens are single-use: call after every auth attempt. */
  reset: () => void;
}

export const AuthCaptcha = forwardRef<AuthCaptchaHandle, { onToken: (token: string | undefined) => void }>(
  function AuthCaptcha({ onToken }, ref) {
    const widget = useRef<TurnstileInstance>(null);

    useImperativeHandle(ref, () => ({
      reset() {
        onToken(undefined);
        widget.current?.reset();
      },
    }));

    if (!captchaEnabled) return null;

    return (
      <Turnstile
        ref={widget}
        siteKey={TURNSTILE_SITE_KEY}
        options={{ theme: "dark", size: "flexible" }}
        onSuccess={onToken}
        onExpire={() => onToken(undefined)}
        onError={() => onToken(undefined)}
      />
    );
  },
);
