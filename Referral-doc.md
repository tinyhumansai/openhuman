# Referral System

## Overview

Referral rewards are paid to the referrer based on real successful payments from referred users.

- Reward rate: `20%` of payment (`2000` basis points)
- Referred user reward: none
- Behavior flag: `RECURRING_REFERRAL_REWARD`
  - `true`: reward every successful eligible payment
  - `false`: reward only once per referral

## Main Rules

- Each user has one unique referral code.
- A user can apply a code only before their first confirmed payment.
- Self-referral is blocked (user id + identity fields).
- Rewarding is idempotent:
  - same payment cannot reward twice
  - non-recurring mode allows only one reward for that referral

## Data Model

### `ReferralCode` (`referralcodes`)

- `userId` (unique)
- `referralCode` (unique)

### `Referral` (`referrals`)

- `referrerId`
- `referredUserId` (unique)
- `referralCode`
- `status`: `pending | converted`
- `sourceIp`, `deviceFingerprint`, `convertedAt`

### `ReferralTransaction` (`referraltransactions`)

- `referralId`, `referrerId`, `referredUserId`
- `sourcePaymentId`, `sourcePaymentGateway`, `sourcePaymentObjectId`
- `paymentAmountUsd`, `rewardAmountUsd`, `rewardRate` (Decimal128)
- `creditTransactionId`
- `idempotencyKey` (unique)

## Migration

Migration file: `src/migrations/1744200000000-referral-system.ts`

What it does:

- creates indexes for referral collections
- backfills missing referral codes for existing users
- backfills `Referral` records from legacy `user.referral.invitedBy`
- supports `users` and `tgusers` collections

### Run migration (non-interactive)

```bash
npx ts-migrate-mongoose up -f src/migrate.ts -a true
```

### Check migration status

```bash
npm run migrate:list
```

### Roll back referral migration (if needed)

```bash
npm run migrate:down
```

## Core Services

- `src/services/referral/referralCodeService.ts`
  - ensures and fetches user referral codes
- `src/services/referral/referralService.ts`
  - apply code, enforce eligibility, return referral stats
- `src/services/referral/referralRewardService.ts`
  - compute reward in cents, award credits, upsert audit transaction, mark referral converted

## API

- `GET /referral/stats`
  - returns code, referral link, totals, and referral rows
- `POST /referral/apply`
  - request: `{ "code": "ABCD1234", "deviceFingerprint": "optional" }`
  - supports `x-device-fingerprint` header

## Payment Integration

Reward processing is triggered on successful payment flows in:

- `src/controllers/payment/coinbase/webhook.ts`
- `src/controllers/payment/stripe/handleWebhook.ts`

## Config

`RECURRING_REFERRAL_REWARD` is read via `nconf`. Truthy values: `true`, `1`, `yes`.

## Tests

Key test: `src/services/referral/__tests__/referralRewardService.test.ts`

- non-recurring behavior
- recurring behavior
- payment idempotency
