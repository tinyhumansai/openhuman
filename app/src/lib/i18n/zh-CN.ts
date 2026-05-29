import zhCN1 from './chunks/zh-CN-1';
import zhCN2 from './chunks/zh-CN-2';
import zhCN3 from './chunks/zh-CN-3';
import zhCN4 from './chunks/zh-CN-4';
import zhCN5 from './chunks/zh-CN-5';
import type { TranslationMap } from './types';

// Simplified Chinese (简体中文) translations. Each chunk maps to chunks/en-N.ts.
// Missing keys fall back to English via I18nContext.resolveEn().
const zhCN: TranslationMap = {
  ...zhCN1,
  ...zhCN2,
  ...zhCN3,
  ...zhCN4,
  ...zhCN5,
  'skills.composio.noApiKeyTitle': '尚未配置 Composio API 密钥',
  'skills.composio.noApiKeyDescription':
    '本地模式使用你自己的 Composio API 密钥。在此连接集成之前，请打开 设置 → 高级 → Composio 添加一个密钥。',
  'skills.composio.noApiKeyCta': '在设置中打开',
  'channels.localManagedUnavailable': '本地用户无法使用托管频道。',
  'rewards.localUnavailable':
    '本地登录不会获得奖励、优惠券或推荐积分。若要累计奖励，请先登出，然后使用 OpenHuman 账号登录。',
  'rewards.localUnavailableCta': '打开账户设置',
  'settings.search.localManagedUnavailable':
    '本地用户无法使用 OpenHuman 托管搜索。请添加你自己的 Parallel、Brave 或 Querit API 密钥以启用网页搜索。',
  'devices.comingSoonDescription': '设备配对即将推出。此页面将用于配对 iPhone 并管理已连接设备。',
  'welcome.continueLocallyExperimental': '本地继续（实验性）',
};

export default zhCN;
