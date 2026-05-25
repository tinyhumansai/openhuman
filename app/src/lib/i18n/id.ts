import id1 from './chunks/id-1';
import id2 from './chunks/id-2';
import id3 from './chunks/id-3';
import id4 from './chunks/id-4';
import id5 from './chunks/id-5';
import type { TranslationMap } from './types';

// Bahasa Indonesia translations. Each chunk maps to chunks/en-N.ts.
// Missing keys fall back to English via I18nContext.resolveEn().
const id: TranslationMap = {
  ...id1,
  ...id2,
  ...id3,
  ...id4,
  ...id5,
  'skills.composio.noApiKeyTitle': 'Belum ada API key Composio yang dikonfigurasi',
  'skills.composio.noApiKeyDescription':
    'Mode lokal menggunakan API key Composio milik Anda sendiri. Buka Pengaturan → Lanjutan → Composio untuk menambahkannya sebelum menghubungkan integrasi di sini.',
  'skills.composio.noApiKeyCta': 'Buka di Pengaturan',
  'channels.localManagedUnavailable': 'Channel terkelola tidak tersedia untuk pengguna lokal.',
  'rewards.localUnavailable':
    'Login lokal tidak mendapatkan reward, kupon, atau kredit referral. Keluar lalu lanjutkan dengan masuk menggunakan akun OpenHuman jika Anda ingin reward dihitung.',
  'rewards.localUnavailableCta': 'Buka Pengaturan Akun',
  'settings.search.localManagedUnavailable':
    'Pencarian OpenHuman Managed tidak tersedia untuk pengguna lokal. Tambahkan API key Parallel atau Brave Anda sendiri untuk mengaktifkan pencarian web.',
  'devices.comingSoonDescription':
    'Pemasangan perangkat akan segera hadir. Halaman ini akan menjadi tempat untuk memasangkan iPhone dan mengelola perangkat yang terhubung.',
  'welcome.continueLocallyExperimental': 'Lanjutkan Secara Lokal (Eksperimental)',
};

export default id;
