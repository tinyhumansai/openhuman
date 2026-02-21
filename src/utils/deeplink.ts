export type WebLoginMethod = 'phone' | 'telegram';

export interface PhoneLoginContext {
  method: 'phone';
  phoneNumber: string;
  countryCode: string;
}

// The shape of the Telegram user object is defined by Telegram.
// We keep it as unknown here and let the backend interpret it.
export interface TelegramLoginContext {
  method: 'telegram';
  telegramUser: unknown;
}

export type WebLoginContext = PhoneLoginContext | TelegramLoginContext;

const DESKTOP_SCHEME = 'alphahuman';

export const buildDesktopDeeplink = (token: string): string => {
  const encoded = encodeURIComponent(token);
  return `${DESKTOP_SCHEME}://auth?token=${encoded}`;
};

export const buildPaymentSuccessDeeplink = (sessionId: string): string => {
  const encoded = encodeURIComponent(sessionId);
  return `${DESKTOP_SCHEME}://payment/success?session_id=${encoded}`;
};

export const buildPaymentCancelDeeplink = (): string => {
  return `${DESKTOP_SCHEME}://payment/cancel`;
};
