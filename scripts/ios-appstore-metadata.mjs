#!/usr/bin/env node
import crypto from "node:crypto";
import { readFile, readdir, stat } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const rootDir = path.resolve(__dirname, "..");
const metadataDir = path.join(rootDir, "fastlane/metadata/en-US");
const screenshotDir = path.join(rootDir, "fastlane/screenshots/en-US");
const apiBase = "https://api.appstoreconnect.apple.com/v1";

const appId = process.env.ASC_APP_ID || "6761229174";
const locale = process.env.ASC_LOCALE || "en-US";
const platform = process.env.ASC_PLATFORM || "IOS";
const screenshotDisplayType =
  process.env.ASC_SCREENSHOT_DISPLAY_TYPE || "APP_IPHONE_67";
const versionString =
  process.env.ASC_VERSION_STRING ||
  JSON.parse(await readFile(path.join(rootDir, "app/package.json"), "utf8"))
    .version;

const keyId = process.env.ASC_KEY_ID;
const issuerId = process.env.ASC_ISSUER_ID;
const keyPath = process.env.ASC_KEY_PATH;

if (!keyId || !issuerId || !keyPath) {
  throw new Error("ASC_KEY_ID, ASC_ISSUER_ID, and ASC_KEY_PATH are required.");
}

function base64Url(input) {
  return Buffer.from(input)
    .toString("base64")
    .replaceAll("+", "-")
    .replaceAll("/", "_")
    .replaceAll("=", "");
}

async function createJwt() {
  const privateKey = await readFile(keyPath, "utf8");
  const now = Math.floor(Date.now() / 1000);
  const header = { alg: "ES256", kid: keyId, typ: "JWT" };
  const payload = {
    iss: issuerId,
    aud: "appstoreconnect-v1",
    iat: now,
    exp: now + 15 * 60,
  };
  const signingInput = `${base64Url(JSON.stringify(header))}.${base64Url(JSON.stringify(payload))}`;
  const signature = crypto.sign("sha256", Buffer.from(signingInput), {
    key: privateKey,
    dsaEncoding: "ieee-p1363",
  });
  return `${signingInput}.${base64Url(signature)}`;
}

const jwt = await createJwt();

async function request(
  method,
  resourcePath,
  body,
  { raw = false, headers = {} } = {},
) {
  const res = await fetch(`${apiBase}${resourcePath}`, {
    method,
    headers: {
      Authorization: `Bearer ${jwt}`,
      Accept: "application/json",
      ...(body && !raw ? { "Content-Type": "application/json" } : {}),
      ...headers,
    },
    body: body ? (raw ? body : JSON.stringify(body)) : undefined,
  });
  const text = await res.text();
  const payload = text ? JSON.parse(text) : null;
  if (!res.ok) {
    const message =
      payload?.errors
        ?.map((e) => `${e.status} ${e.code}: ${e.detail}`)
        .join("\n") || text;
    throw new Error(`${method} ${resourcePath} failed: ${message}`);
  }
  return payload;
}

async function uploadOperation(operation, fileBuffer) {
  const headers = Object.fromEntries(
    (operation.requestHeaders || []).map((h) => [h.name, h.value]),
  );
  const offset = Number(operation.offset || 0);
  const length = Number(operation.length || fileBuffer.length);
  const chunk = fileBuffer.subarray(offset, offset + length);
  const res = await fetch(operation.url, {
    method: operation.method,
    headers,
    body: chunk,
  });
  if (!res.ok) {
    throw new Error(
      `asset upload failed: ${res.status} ${res.statusText} ${await res.text()}`,
    );
  }
}

async function textFile(name) {
  return (await readFile(path.join(metadataDir, name), "utf8")).trim();
}

async function firstPage(resourcePath) {
  const payload = await request("GET", resourcePath);
  return payload.data || [];
}

async function getOrCreateAppInfoLocalization() {
  const appInfos = await firstPage(`/apps/${appId}/appInfos?limit=10`);
  if (!appInfos.length) {
    throw new Error(`No appInfos found for app ${appId}.`);
  }
  const appInfo = appInfos[0];
  const existing = await firstPage(
    `/appInfos/${appInfo.id}/appInfoLocalizations?limit=50`,
  );
  const match = existing.find((item) => item.attributes?.locale === locale);
  if (match) return match;

  const created = await request("POST", "/appInfoLocalizations", {
    data: {
      type: "appInfoLocalizations",
      attributes: { locale, name: await textFile("name.txt") },
      relationships: {
        appInfo: { data: { type: "appInfos", id: appInfo.id } },
      },
    },
  });
  return created.data;
}

async function updateAppInfoLocalization() {
  const localization = await getOrCreateAppInfoLocalization();
  await request("PATCH", `/appInfoLocalizations/${localization.id}`, {
    data: {
      type: "appInfoLocalizations",
      id: localization.id,
      attributes: {
        name: await textFile("name.txt"),
        subtitle: await textFile("subtitle.txt"),
        privacyPolicyUrl: await textFile("privacy_url.txt"),
      },
    },
  });
  console.log(
    `[ios-appstore-metadata] updated app info localization ${locale}`,
  );
}

async function getOrCreateAppStoreVersion() {
  const versions = await firstPage(
    `/apps/${appId}/appStoreVersions?filter[platform]=${platform}&limit=20`,
  );
  const editableStates = new Set([
    "PREPARE_FOR_SUBMISSION",
    "DEVELOPER_REJECTED",
    "REJECTED",
    "METADATA_REJECTED",
    "INVALID_BINARY",
  ]);
  const existing =
    versions.find((v) => v.attributes?.versionString === versionString) ||
    versions.find((v) => editableStates.has(v.attributes?.appStoreState));
  if (existing) return existing;

  const created = await request("POST", "/appStoreVersions", {
    data: {
      type: "appStoreVersions",
      attributes: {
        platform,
        versionString,
        copyright: await textFile("copyright.txt"),
      },
      relationships: {
        app: { data: { type: "apps", id: appId } },
      },
    },
  });
  console.log(
    `[ios-appstore-metadata] created ${platform} version ${versionString}`,
  );
  return created.data;
}

async function getOrCreateVersionLocalization(version) {
  const existing = await firstPage(
    `/appStoreVersions/${version.id}/appStoreVersionLocalizations?limit=50`,
  );
  const match = existing.find((item) => item.attributes?.locale === locale);
  if (match) return match;

  const created = await request("POST", "/appStoreVersionLocalizations", {
    data: {
      type: "appStoreVersionLocalizations",
      attributes: { locale },
      relationships: {
        appStoreVersion: { data: { type: "appStoreVersions", id: version.id } },
      },
    },
  });
  return created.data;
}

async function updateVersionLocalization() {
  const version = await getOrCreateAppStoreVersion();
  const localization = await getOrCreateVersionLocalization(version);
  await request("PATCH", `/appStoreVersionLocalizations/${localization.id}`, {
    data: {
      type: "appStoreVersionLocalizations",
      id: localization.id,
      attributes: {
        description: await textFile("description.txt"),
        keywords: await textFile("keywords.txt"),
        marketingUrl: await textFile("marketing_url.txt"),
        promotionalText: await textFile("promotional_text.txt"),
        supportUrl: await textFile("support_url.txt"),
      },
    },
  });
  console.log(
    `[ios-appstore-metadata] updated version localization ${locale} for ${version.attributes.versionString}`,
  );
  return localization;
}

async function deleteExistingScreenshotSet(localization) {
  const sets = await firstPage(
    `/appStoreVersionLocalizations/${localization.id}/appScreenshotSets?filter[screenshotDisplayType]=${screenshotDisplayType}&limit=50&include=appScreenshots`,
  );
  for (const set of sets) {
    await request("DELETE", `/appScreenshotSets/${set.id}`);
    console.log(
      `[ios-appstore-metadata] deleted existing screenshot set ${set.id}`,
    );
  }
}

async function createScreenshotSet(localization) {
  const created = await request("POST", "/appScreenshotSets", {
    data: {
      type: "appScreenshotSets",
      attributes: { screenshotDisplayType },
      relationships: {
        appStoreVersionLocalization: {
          data: { type: "appStoreVersionLocalizations", id: localization.id },
        },
      },
    },
  });
  console.log(
    `[ios-appstore-metadata] created screenshot set ${screenshotDisplayType}`,
  );
  return created.data;
}

async function uploadScreenshot(set, filePath) {
  const fileName = path.basename(filePath);
  const fileBuffer = await readFile(filePath);
  const fileInfo = await stat(filePath);
  const checksum = crypto.createHash("md5").update(fileBuffer).digest("hex");

  const reservation = await request("POST", "/appScreenshots", {
    data: {
      type: "appScreenshots",
      attributes: { fileName, fileSize: fileInfo.size },
      relationships: {
        appScreenshotSet: { data: { type: "appScreenshotSets", id: set.id } },
      },
    },
  });

  const operations = reservation.data.attributes.uploadOperations || [];
  for (const operation of operations) {
    await uploadOperation(operation, fileBuffer);
  }

  await request("PATCH", `/appScreenshots/${reservation.data.id}`, {
    data: {
      type: "appScreenshots",
      id: reservation.data.id,
      attributes: {
        uploaded: true,
        sourceFileChecksum: checksum,
      },
    },
  });
  console.log(`[ios-appstore-metadata] uploaded ${fileName}`);
}

async function uploadScreenshots(localization) {
  const files = (await readdir(screenshotDir))
    .filter((file) => file.endsWith(".png"))
    .sort()
    .map((file) => path.join(screenshotDir, file));
  if (!files.length) {
    throw new Error(`No screenshots found in ${screenshotDir}`);
  }

  await deleteExistingScreenshotSet(localization);
  const set = await createScreenshotSet(localization);
  for (const file of files) {
    await uploadScreenshot(set, file);
  }
}

await updateAppInfoLocalization();
const versionLocalization = await updateVersionLocalization();
await uploadScreenshots(versionLocalization);

console.log(
  "[ios-appstore-metadata] metadata and screenshots submitted to App Store Connect.",
);
