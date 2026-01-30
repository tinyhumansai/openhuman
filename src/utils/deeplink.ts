export type WebLoginMethod = "phone" | "telegram";

export interface PhoneLoginContext {
  method: "phone";
  phoneNumber: string;
  countryCode: string;
}

// The shape of the Telegram user object is defined by Telegram.
// We keep it as unknown here and let the backend interpret it.
export interface TelegramLoginContext {
  method: "telegram";
  telegramUser: unknown;
}

export type WebLoginContext = PhoneLoginContext | TelegramLoginContext;

const DESKTOP_SCHEME = "alphahuman";

export const buildDesktopDeeplink = (token: string): string => {
  const encoded = encodeURIComponent(token);
  return `${DESKTOP_SCHEME}://auth?token=${encoded}`;
};

/**
 * Call the backend to complete the web login and obtain a short-lived token
 * that can be handed off to the desktop app via deeplink.
 *
 * This expects a backend endpoint at POST /api/auth/web-complete that
 * validates the current web session and returns `{ loginToken: string }`.
 */
export const completeWebLoginAndOpenDesktop = async (
  context: WebLoginContext,
): Promise<void> => {
  const response = await fetch("/api/auth/web-complete", {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
    },
    body: JSON.stringify(context),
    credentials: "include",
  });

  if (!response.ok) {
    throw new Error("Failed to complete web login for desktop handoff");
  }

  const data = (await response.json()) as { loginToken?: string };
  if (!data.loginToken) {
    throw new Error("Backend response did not include a loginToken");
  }

  const deeplink = buildDesktopDeeplink(data.loginToken);
  window.location.href = deeplink;
};
