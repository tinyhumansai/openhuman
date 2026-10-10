<div dir="ltr">

<h1 align="center">OpenHuman</h1>

</div>

<div dir="rtl" lang="ur">

<p align="center">
 <strong>سب سے تیز، سستا اور موثر اوپن سورس ایجنٹ ہارنیس۔ صرف $10 کے VPS پر 500 سے زیادہ ایجنٹ چلائیں۔</strong><br/>
 عام لوگوں کے لیے ڈیسک ٹاپ ایپ۔ ڈیولپرز کے لیے Rust لائبریری۔
</p>

</div>

<div dir="ltr">

<p align="center">
 <img src="https://img.shields.io/badge/status-early%20beta-orange" alt="ابتدائی آزمائشی نسخہ" />
 <a href="https://github.com/tinyhumansai/openhuman/releases/latest"><img src="https://img.shields.io/github/v/release/tinyhumansai/openhuman?label=latest" alt="تازہ ترین نسخہ" /></a>
 <a href="https://github.com/tinyhumansai/openhuman/stargazers"><img src="https://img.shields.io/github/stars/tinyhumansai/openhuman?style=flat" alt="گٹ ہب ستارے" /></a>
 <a href="../LICENSE"><img src="https://img.shields.io/github/license/tinyhumansai/openhuman" alt="لائسنس" /></a>
 <a href="https://github.com/tinyhumansai/openhuman-benchmarks"><img src="https://img.shields.io/badge/benchmarks-public-brightgreen" alt="عوامی بینچ مارکس" /></a>
</p>

<p align="center">
 <a href="https://tinyhumans.gitbook.io/openhuman/">دستاویزات</a> ·
 <a href="https://tinyhumans.gitbook.io/openhuman/developing/quickstart">Rust کوئیک اسٹارٹ</a> ·
 <a href="https://tinyhumansai.github.io/openhuman-benchmarks/">بینچ مارکس</a> ·
 <a href="https://github.com/tinyhumansai/openhuman/discussions">گفتگو</a> ·
 <a href="https://guild.tinyhumans.ai/">Discord</a> ·
 <a href="https://www.reddit.com/r/tinyhumansai/">Reddit</a> ·
 <a href="https://x.com/intent/follow?screen_name=tinyhumansai">X</a> ·
 <a href="https://x.com/intent/follow?screen_name=senamakel">@senamakel (خالق)</a>
</p>

<p align="center">
 <img src="./demo.gif" alt="OpenHuman ڈیسک ٹاپ ایپ کی ایک جھلک" />
</p>

<p align="center">
	<a href="https://trendshift.io/repositories/23680" target="_blank">
		<img src="https://trendshift.io/api/badge/repositories/23680" alt="tinyhumansai%2Fopenhuman | Trendshift" width="250" height="55"/>
	</a>
	<a href="https://www.producthunt.com/products/openhuman?embed=true&amp;utm_source=badge-top-post-badge&amp;utm_medium=badge&amp;utm_campaign=badge-openhuman" target="_blank" rel="noopener noreferrer">
		<img alt="OpenHuman on Product Hunt" width="250" height="54" src="https://api.producthunt.com/widgets/embed-image/v1/top-post-badge.svg?post_id=1136902&amp;theme=light&amp;period=daily&amp;t=1778916022823">
	</a>
	<a href="https://www.producthunt.com/products/openhuman?embed=true&amp;utm_source=badge-top-post-badge&amp;utm_medium=badge&amp;utm_campaign=badge-openhuman" target="_blank" rel="noopener noreferrer">
		<img alt="OpenHuman on Product Hunt" width="250" height="54" src="https://api.producthunt.com/widgets/embed-image/v1/top-post-badge.svg?post_id=1136902&amp;theme=light&amp;period=weekly&amp;t=1779351403565">
	</a>
</p>

<p align="center">
  🇺🇸 <a href="../README.md">English</a> | 🇸🇦 <a href="./README.ar.md">العربية</a> | 🇨🇳 <a href="./README.zh-CN.md">简体中文</a> | 🇯🇵 <a href="./README.ja-JP.md">日本語</a> | 🇰🇷 <a href="./README.ko.md">한국어</a> | 🇩🇪 <a href="./README.de.md">Deutsch</a> | 🇹🇷 <a href="./README.tr.md">Türkçe</a> | 🇵🇰 <a href="./README.ur-pk.md"><strong>اردو</strong></a>
</p>

</div>

<div dir="rtl" lang="ur">

> [!NOTE]
> 🎉 لانچ کے ایک ہفتے کے اندر، OpenHuman مسلسل نو دن تک GitHub پر **نمبر ایک ٹرینڈنگ ریپوزٹری** رہا۔

> **ابتدائی آزمائشی نسخہ۔** OpenHuman پر فعال کام جاری ہے، اس لیے کچھ خامیوں کی توقع رکھیں۔

---

## انسٹال کریں

سب سے آسان طریقہ یہ ہے کہ ڈیسک ٹاپ ایپ [tinyhumans.ai/openhuman](https://tinyhumans.ai/openhuman?utm_source=github&utm_medium=readme) یا [تازہ ترین ریلیز](https://github.com/tinyhumansai/openhuman/releases/latest) سے ڈاؤن لوڈ کریں۔ macOS کے لیے `.dmg`، Windows کے لیے `.msi` یا `.exe`، اور Linux کے لیے `.deb` یا `.AppImage` موجود ہے۔

ٹرمینل پسند ہے؟ انسٹال اسکرپٹ آپ کے سسٹم کے لیے درست پیکیج چنتی ہے، اس کا چیک سم جانچتی ہے اور اسے انسٹال کر دیتی ہے:

</div>

<div dir="ltr">

```bash
# macOS and Linux
curl -fsSL https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.sh | bash
```

```powershell
# Windows (PowerShell)
irm https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.ps1 | iex
```

</div>

<div dir="rtl" lang="ur">

macOS اور Linux کی اسکرپٹ کیا کرے گی، یہ پہلے دیکھنے کے لیے کمانڈ کے آخر میں `bash -s -- --dry-run` لگائیں۔ دوسرے اختیارات اور مسائل کا حل [INSTALL.md](../INSTALL.md) میں ہے۔

---

## OpenHuman ہی کیوں؟

زیادہ تر ایجنٹ ہارنیس ہر ایجنٹ کے لیے ایک بھاری پروسیس چلاتے ہیں اور ہر کال پر بڑا پرامپٹ دوبارہ بھیجتے ہیں۔ OpenHuman وہی کام بہت کم وسائل میں کرتا ہے۔ یہ ایجنٹوں کے بڑے بیڑے کے لیے بنایا گیا واحد مکمل فیچرز والا اوپن سورس ہارنیس ہے: 500 ایجنٹ $10 کے سرور پر سما جاتے ہیں۔

<table>

<tr>

<td width="50%" valign="top">

<h3>تیز</h3>

<p><a href="https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/results/swe-x86-1/summary.md">بینچ مارک نتائج</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/performance">کولڈ اسٹارٹ کے اعداد و شمار</a></p>

<p>کوڈنگ کے کام تقریباً 20 سیکنڈ میں مکمل کرتا ہے، جو ہمارے آزمائے ہوئے سات AI ایجنٹ ٹولز میں سب سے تیز ہے۔ ایک سیکنڈ کے دسویں حصے میں شروع ہو جاتا ہے۔</p>

</td>

<td width="50%" valign="top">

<h3>سستا</h3>

<p><a href="https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/results/swe-x86-1/summary.md">لاگت اور ٹوکن ڈیٹا</a> · <a href="https://tinyhumans.gitbook.io/openhuman/features/token-compression">کمپریشن کیسے کام کرتی ہے</a></p>

<p>عام ایجنٹ ٹول کے مقابلے میں 2.6 گنا کم ٹوکن استعمال کرتا ہے، اور ہمارے ٹیسٹ میں اس کا کل بل سب سے کم رہا۔</p>

</td>

</tr>

<tr>

<td width="50%" valign="top">

<h3>بڑے پیمانے پر موثر</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/performance">بیڑے کی پیمائشیں</a> · <a href="https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/profile/docs/library-benchmarking.md">طریقہ کار</a></p>

<p>عام ایجنٹ ٹول کے مقابلے میں تقریباً 8 گنا کم میموری اور CPU استعمال کرتا ہے۔ $10 کے سرور پر 500 سے زیادہ ایجنٹ چلائیں۔</p>

</td>

<td width="50%" valign="top">

<h3>ڈیولپرز کے لیے بنایا گیا</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/quickstart">Rust کوئیک اسٹارٹ</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/embed">ایمبیڈنگ گائیڈ</a> · <a href="../crates/openhuman-embed/examples">مثالیں</a></p>

<p>اسے Rust لائبریری کے طور پر استعمال کریں: ایجنٹ کو کسی بھی عام فنکشن کی طرح کال کریں، یا ایک چھوٹے سرور سے پورا بیڑا چلائیں۔</p>

</td>

</tr>

</table>

</div>

<div dir="ltr">

<p align="center">
 <img src="./oh-v-hermes.gif" alt="Hermes بمقابلہ OpenHuman" />
</p>

</div>

<div dir="rtl" lang="ur">

<p align="center"><em>ایک ہی پرامپٹ، ساتھ ساتھ: "ایجنٹ ہارنیس کے بارے میں ایک اینیمی کہانی لکھو، اسے HTML صفحے میں لکھو اور میرے لیے کھول دو۔"<br/>OpenHuman نے 20 سیکنڈ میں مکمل کیا، 15k ٹوکن اور $0.0054 لاگت کے ساتھ۔ Hermes نے 9 منٹ 40 سیکنڈ لیے، 37k ٹوکن اور $0.0082 لاگت کے ساتھ۔</em></p>

---

## بڑی ایجادات

زیادہ تر ایجنٹ ہارنیس ایک سادہ چکر ہیں: سب کچھ ماڈل کو بھیجو، انتظار کرو، دہراؤ۔ ایک ایجنٹ کے لیے یہ ٹھیک ہے، لیکن جلد ہی سست اور مہنگا ہو جاتا ہے، اور سینکڑوں ایجنٹ چلانے پر ناکام ہو جاتا ہے۔

OpenHuman ان حصوں کو نئے سرے سے سوچتا ہے جن پر سب سے زیادہ خرچ آتا ہے: AI کو کتنا متن پڑھنا پڑتا ہے، وہ صحیح ٹول کیسے ڈھونڈتا ہے، فیچرز کیسے لوڈ ہوتے ہیں، اور پورے نظام کو کتنی مشین درکار ہے۔ نیچے دیے گئے چھ خیالات ہی اوپر بیان کی گئی رفتار اور بچت کی وجہ ہیں۔ تفصیل کے لیے ہر کارڈ دستاویزات سے جڑا ہے۔

<table>

<tr>

<td width="50%" valign="top">

<h3>RLM ٹوکن کمپریشن</h3>

<p><a href="https://arxiv.org/abs/2512.24601">RLM مقالہ</a> · <a href="https://tinyhumans.gitbook.io/openhuman/features/token-compression">یہ کیسے کام کرتا ہے</a></p>

<p><a href="https://arxiv.org/abs/2512.24601">Recursive Language Models</a> پر مبنی۔ ٹول کے بڑے نتائج AI کے پڑھنے سے پہلے کمپریس ہو جاتے ہیں۔ بہت بڑے نتائج کے لیے AI کو ایک ہینڈل ملتا ہے جس میں وہ سب کچھ پڑھنے کے بجائے تلاش کر سکتا ہے۔ کچھ بھی ضائع نہیں ہوتا۔</p>

</td>

<td width="50%" valign="top">

<h3>Jev: فوری اور درست ٹول سرچ</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/jev">Jev</a> · <a href="./plans/jev-tool-search-baseline.md">پیمائشیں</a></p>

<p>Jev ایک چھوٹا ماڈل ہے جو 1,215 ٹولز میں سے صحیح ٹول ڈھونڈتا ہے۔ صحیح ٹول 86.8% مرتبہ اس کی اوپر کی تجاویز میں ہوتا ہے، جبکہ کی ورڈ سرچ میں یہ شرح 70.5% ہے۔</p>

</td>

</tr>

<tr>

<td width="50%" valign="top">

<h3>متحد Rust بس</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/loadable-modules">پلگ اِن کیسے کام کرتے ہیں</a></p>

<p>سرچ، دستاویزات یا آواز جیسا ہر فیچر ایک ہی Rust بس میں جڑتا ہے، یہ خیال <a href="https://www.freedesktop.org/wiki/Software/dbus/">Linux سسٹم بس</a> سے لیا گیا ہے۔ فیچر صرف ضرورت پڑنے پر لوڈ ہوتا ہے، اور اگر کوئی ایک اٹک جائے تو باقی کام کرتے رہتے ہیں۔</p>

</td>

<td width="50%" valign="top">

<h3>گہرائی سے مربوط میموری</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/features/memory">میموری کیسے کام کرتی ہے</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/engines">میموری انجن</a></p>

<p>میموری پہلے سے شامل ہے۔ ہر باری سے پہلے OpenHuman ٹوکن کی حد کے اندر صرف کام کی چیزیں چنتا ہے اور حوالوں کے ساتھ AI کو دے دیتا ہے۔ یہ بغیر کسی سیٹ اپ کے چلتی ہے، اور ایک سیٹنگ سے آپ کوئی دوسرا میموری انجن لگا سکتے ہیں۔</p>

</td>

</tr>

<tr>

<td width="50%" valign="top">

<h3>براؤزر اور ڈیسک ٹاپ پر فوری کنٹرول</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/features/native-tools/browser-and-computer">براؤزر اور کمپیوٹر کنٹرول</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/jev">Jev</a></p>

<p>ایجنٹ ایک اصل براؤزر اور آپ کی ڈیسک ٹاپ ایپس استعمال کرتا ہے۔ Jev اسکرین پر موجود بٹنوں میں سے ہر کلک چنتا ہے، اسکرین شاٹس کے بغیر۔ کسی بھی ادائیگی سے پہلے رک جاتا ہے۔</p>

</td>

<td width="50%" valign="top">

<h3>پروگرام کے قابل Rust کور</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/quickstart">Rust کوئیک اسٹارٹ</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/performance">کارکردگی</a></p>

<p>پورا ہارنیس ایک ہی پروسیس میں کمپائل شدہ Rust ہے، اس لیے ایک سیکنڈ کے دسویں حصے میں شروع ہوتا ہے اور ہلکا رہتا ہے: ہمارے کوڈنگ کاموں پر زیادہ سے زیادہ 68 MB۔ یہی کور ایک لائبریری بھی ہے۔ اپنے Rust کوڈ سے ایجنٹ کو کال کریں، یا ایک چھوٹے سرور پر سینکڑوں ایجنٹ ساتھ ساتھ چلائیں۔</p>

</td>

</tr>

</table>

---

## بینچ مارکس

</div>

<div dir="ltr">

<p align="center">
 <picture>
  <source media="(prefers-color-scheme: dark)" srcset="../gitbooks/.gitbook/assets/benchmarks/swe-x86-1-dark.png" />
  <source media="(prefers-color-scheme: light)" srcset="../gitbooks/.gitbook/assets/benchmarks/swe-x86-1.png" />
  <img alt="SWE-bench Verified رن swe-x86-1: OpenHuman بمقابلہ چھ دوسرے ہارنیس، دس پینلز میں" src="../gitbooks/.gitbook/assets/benchmarks/swe-x86-1.png" />
 </picture>
</p>

</div>

<div dir="rtl" lang="ur">

ہم نے سات ہارنیس کو ایک ہی SWE-bench کاموں پر، ایک ہی ماڈل، API کلید اور کنٹینر کے ساتھ چلایا، اور ہر کال ناپی۔ سیٹ اپ اور نتائج [openhuman-benchmarks](https://github.com/tinyhumansai/openhuman-benchmarks) میں عوامی ہیں، اس لیے کوئی بھی انہیں دوبارہ چلا سکتا ہے۔ تازہ ترین رن [`swe-x86-1`](https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/results/swe-x86-1/summary.md) ہے، جس میں دس کام ہیں۔

OpenHuman نے اپنے کام اوسط وقت کے آدھے سے بھی کم میں مکمل کیے، 2.6 گنا کم ٹوکن اور آٹھویں حصے کی میموری اور CPU کے ساتھ۔ اس نے 114 ماڈل کالز کیں جبکہ دوسروں نے 134 سے 212 کیں، اور اس کے پورے رن پر $0.05 لاگت آئی جبکہ دوسروں پر $0.08 سے $0.35۔

یہ ہر قدم پر کم کام کر کے جیتتا ہے۔ کور ایک ہی پروسیس میں کمپائل شدہ Rust ہے، Node یا Python نہیں، اس لیے ماڈل کالز کے درمیان یہ بہت کم میموری اور تقریباً نہ ہونے کے برابر CPU استعمال کرتا ہے۔ یہ ساتوں میں سب سے چھوٹا پرامپٹ بھیجتا ہے، اپنے 20 ٹولز سمیت 4.6k ٹوکن، اس لیے ہر کال سستی ہوتی ہے اور جلد واپس آتی ہے (اوسط 1.65 سیکنڈ، یہ بھی سب سے تیز)۔ اسے جواب تک پہنچنے کے لیے کم کالز بھی چاہیے ہوتی ہیں، اور زیادہ تر بچت یہیں سے آتی ہے۔

| فی حل شدہ کام | OpenHuman (اوسط کے مقابلے میں) | باقی چھ کا اوسط | باقی چھ میں بہترین |
| --- | --- | --- | --- |
| کل وقت (p50) | **19.8 s** (2.3 گنا تیز) | 46.5 s | 28.4 s (OpenCode) |
| ٹوکن | **167k** (2.6 گنا کم) | 435k | 370k (Codex) |
| لاگت | **$0.0077** (-36%) | $0.012 | $0.0077 (OpenCode، برابر) |
| زیادہ سے زیادہ میموری | **68 MB** (7.7 گنا کم) | 523 MB | 123 MB (Codex) |
| CPU وقت | **1.3 s** (8 گنا کم) | 10.4 s | 2.9 s (DeepSeek Harness) |
| سسٹم پرامپٹ | **4.6k ٹوکن** (-40%) | 7.7k | 6.2k (DeepSeek Harness) |

---

## صارفین کے لیے

اگر آپ Claude Code، Codex، OpenClaw یا Hermes استعمال کرتے ہیں تو آپ یہ خیالات پہلے سے جانتے ہیں: ٹولز کے ساتھ ایجنٹ لوپ، MCP، اسکلز، BYOK ماڈلز اور میموری۔ OpenHuman میں یہ سب موجود ہیں، ڈیسک ٹاپ ایپ، ٹرمینل ایپ یا بغیر اسکرین کے سرور میں۔

| صلاحیت | OpenHuman میں کیا شامل ہے |
| --- | --- |
| ترتیب دینے کے قابل میموری | [آپ کی فائلوں، ریپوز، فیڈز اور ایپس پر بلٹ اِن میموری](https://tinyhumans.gitbook.io/openhuman/features/memory)، ہر باری سے پہلے حوالوں کے ساتھ یاد کی جاتی ہے، آپ کے چنے ہوئے میموری انجن پر |
| وائس ایجنٹس | ایک [لائیو وائس ایجنٹ](https://tinyhumans.gitbook.io/openhuman/features/native-tools/voice) جسے آپ بات کے بیچ میں روک سکتے ہیں، ساتھ ڈکٹیشن اور بولے گئے جوابات |
| کمپیوٹر اور براؤزر کنٹرول | [اصل Chrome براؤزر اور آپ کی ڈیسک ٹاپ ایپس چلاتا ہے](https://tinyhumans.gitbook.io/openhuman/features/native-tools/browser-and-computer)، ایسی کسی بھی چیز سے پہلے پوچھتا ہے جو واپس نہ ہو سکے |
| سرچ | [سرچ انجن اور سرچ ایجنٹس](https://tinyhumans.gitbook.io/openhuman/features/native-tools/web-search): Exa اور Gemini شامل ہیں، Brave، Tavily، Parallel اور مزید آپ کی اپنی کلید کے ساتھ، حوالوں کے ساتھ مستند جوابات اور Deep Research |
| ٹولز | [نیٹو ٹولز](https://tinyhumans.gitbook.io/openhuman/features/native-tools): شیل اور کوڈر، اسکریپر، دستاویزات، تصویر اور ویڈیو بنانا، cron |
| MCP اور اسکلز | [MCP سرورز اور اسکل بنڈلز](https://tinyhumans.gitbook.io/openhuman/features/integrations/mcp-and-skills) |
| OAuth انٹیگریشنز | [Composio کے ذریعے 119 ایپس](https://tinyhumans.gitbook.io/openhuman/features/integrations)، [ٹرگرز](https://tinyhumans.gitbook.io/openhuman/features/integrations/triggers) کے ساتھ جو کچھ ہونے پر ایجنٹ کو شروع کر دیتے ہیں |
| ماڈلز | [لوکل ماڈلز (Ollama، LM Studio، MLX) اور 26 فراہم کنندگان کے لیے BYOK](https://tinyhumans.gitbook.io/openhuman/features/model-routing/local-and-byok-models)، [خودکار ماڈل روٹنگ](https://tinyhumans.gitbook.io/openhuman/features/model-routing) کے ساتھ |
| چینلز | ایجنٹ کے فرنٹ اینڈ کے طور پر [14 میسجنگ چینلز](https://tinyhumans.gitbook.io/openhuman/features/channels): Telegram، Discord، iMessage، ای میل اور مزید |
| ورک فلوز | [پائیدار ورک فلو گراف](https://tinyhumans.gitbook.io/openhuman/features/workflows): cron، ایونٹ یا دستی ٹرگرز، منظوری کے مراحل، وقفے کے بعد دوبارہ شروع |
| حفاظت | [منظوری کا گیٹ](https://tinyhumans.gitbook.io/openhuman/features/approval-gate)، [سینڈ باکس میں عمل](https://tinyhumans.gitbook.io/openhuman/features/privacy-and-security) (OS جیل یا Docker)، صرف مقامی رن کے لیے [پرائیویسی موڈ](https://tinyhumans.gitbook.io/openhuman/features/privacy-mode)، رازوں کے لیے [OS کی رنگ](https://tinyhumans.gitbook.io/openhuman/features/os-keyring-and-secret-storage) |
| استعمال کی ٹریکنگ | [فی کال لاگت اور ٹوکن کا استعمال](https://tinyhumans.gitbook.io/openhuman/features/billing-and-usage)، ساتھ دوبارہ چلانے کے قابل رن جرنلز |

[شروعات](https://tinyhumans.gitbook.io/openhuman/overview/getting-started) یا [گائیڈز](https://tinyhumans.gitbook.io/openhuman/guides) سے آغاز کریں۔

---

## ڈیولپرز کے لیے

OpenHuman لائبریری کو ترجیح دینے والا ہارنیس ہے۔ اسے اپنے Rust پروجیکٹ میں شامل کریں اور ایجنٹ کو کسی بھی عام فنکشن کی طرح کال کریں، کوئی سائیڈ کار یا ڈیمن چلانے کی ضرورت نہیں۔ ایک رن ٹائم سینکڑوں ایجنٹ سنبھال سکتا ہے، ہر ایک کا اپنا ماڈل، ٹولز، میموری اور سینڈ باکس۔ ان میں سے 500 $10 کے VPS پر سما جاتے ہیں، اس لیے ہر گاہک کے لیے ایک ایجنٹ والی پروڈکٹ کلسٹر کے بغیر شروع ہو سکتی ہے۔

</div>

<div dir="ltr">

```rust
use openhuman_embed::{Access, Harness, Provider, Workspace};

let agent = Harness::builder()
    .provider(Provider::openai_compatible("https://api.openai.com/v1", "sk-...").model("gpt-5"))
    .workspace(Workspace::Ephemeral)
    .access(Access::readonly())
    .build()
    .await?;

let reply = agent.run("Summarize what you can see in this directory.").await?;
println!("{}", reply.reply);
```

</div>

<div dir="rtl" lang="ur">

اگلا قدم: [Rust کوئیک اسٹارٹ](https://tinyhumans.gitbook.io/openhuman/developing/quickstart)، [ایمبیڈنگ گائیڈ](https://tinyhumans.gitbook.io/openhuman/developing/embed) اور [ڈیولپر دستاویزات](https://tinyhumans.gitbook.io/openhuman/developing)۔

---

## موازنہ کیسا رہتا ہے؟

یہاں OpenHuman کو ان ہارنیس کے ساتھ رکھا گیا ہے جو زیادہ تر لوگ پہلے سے استعمال کرتے ہیں۔ اوپر کی چار قطاریں ہمارے [عوامی بینچ مارک](https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/results/swe-x86-1/summary.md) سے ہیں: SWE-bench کے دس کوڈنگ کام، ہر ہارنیس کے لیے ایک ہی ماڈل اور کنٹینر۔ ہر عدد فی حل شدہ کام ہے۔ باقی قطاریں فیچرز کا موازنہ کرتی ہیں۔

مختصر یہ کہ باقی ایک شخص کے ایک ایجنٹ کے ساتھ کام کرنے کے لیے بہترین ہیں۔ OpenHuman یہ بھی کرتا ہے، اور ساتھ ہی یہ وہ اوپن سورس آپشن ہے جسے آپ لائبریری کے طور پر ایمبیڈ کر کے بیڑے تک بڑھا سکتے ہیں۔ پروڈکٹس تیزی سے بدلتی ہیں، اس لیے فیصلے سے پہلے ہر پروجیکٹ کو خود دیکھ لیں۔

|                         | Claude Code          | Codex                | OpenClaw             | Hermes Agent         | OpenHuman                                |
| ----------------------- | -------------------- | -------------------- | -------------------- | -------------------- | ---------------------------------------- |
| **اوسط کام کا وقت**     | 37.8 s               | 36.1 s               | 62 s                 | 58.5 s               | 🚀 19.8 s                                 |
| **فی کام اوسط ٹوکن**    | 428k                 | 370k                 | 442k                 | 482k                 | 🚀 167k                                   |
| **فی کام زیادہ سے زیادہ میموری** | 231 MB               | 123 MB               | 1.57 GB              | 816 MB               | 🚀 68 MB                                  |
| **فی کام اوسط لاگت**    | $0.04                | $0.02                | $0.01                | $0.0082              | 🚀 $0.0077                                |
| **اوپن سورس**           | 🚫 ملکیتی            | ✅ Apache-2.0        | ✅ MIT               | ✅ MIT               | ✅ GPL-3.0                               |
| **ایجنٹ بیڑے**          | ⚠️ ہر ایجنٹ کا پروسیس | ⚠️ ہر ایجنٹ کا پروسیس | ⚠️ ہر ایجنٹ کا پروسیس | ⚠️ ہر ایجنٹ کا پروسیس | 🚀 $10 کے VPS پر 500 ایجنٹ               |
| **ایمبیڈ ہونے والی لائبریری** | ⚠️ CLI پر SDK        | ⚠️ CLI پر SDK        | 🚫 کوئی نہیں         | ⚠️ Python پیکیج      | 🚀 ٹائپ شدہ Rust API                      |
| **میموری**              | ⚠️ میموری فائلیں     | ⚠️ میموری فائلیں     | ⚠️ پلگ اِن پر منحصر  | ✅ خود سیکھنے والی   | 🚀 ہر باری حوالوں کے ساتھ یاد کی جاتی    |
| **انٹیگریشنز**          | ✅ MCP               | ✅ MCP               | ⚠️ خود لائیں         | ⚠️ خود لائیں         | 🚀 119 OAuth ایپس، MCP، اسکلز            |
| **میسجنگ چینلز**        | 🚫 کوئی نہیں         | 🚫 کوئی نہیں         | ✅ کئی               | ✅ چند               | ✅ 14، بشمول ای میل                      |
| **براؤزر اور ڈیسک ٹاپ** | ⚠️ MCP کے ذریعے براؤزر | ⚠️ MCP کے ذریعے براؤزر | ✅ براؤزر            | ✅ براؤزر            | ✅ براؤزر اور ڈیسک ٹاپ ایپس              |
| **ورک فلوز**            | 🚫 کوئی نہیں         | 🚫 کوئی نہیں         | ⚠️ اسکرپٹس           | ⚠️ اسکرپٹس           | 🚀 بصری، ایجنٹ کے بنائے مسودے            |
| **ماڈل کا انتخاب**      | ⚠️ Anthropic ماڈلز   | ⚠️ پہلے OpenAI       | ✅ کوئی بھی          | ✅ کوئی بھی          | ✅ کوئی بھی، بلٹ اِن روٹنگ کے ساتھ       |

---

## شراکت

[`CONTRIBUTING.md`](../CONTRIBUTING.md) پڑھیں، یا کسی AI کوڈنگ ایجنٹ سے [اس پرامپٹ](./CONTRIBUTING-BEGINNERS.md#optional--let-an-ai-coding-agent-guide-you) کے ذریعے رہنمائی لیں۔

1. Git، Node.js 24+، pnpm 10.10.0، Rust 1.96.1 (`rustfmt` اور `clippy` کے ساتھ)، CMake، Ninja، ripgrep، اور اپنے پلیٹ فارم کے ڈیسک ٹاپ بلڈ کے تقاضے انسٹال کریں۔
2. ریپو کو فورک کر کے کلون کریں۔ `git submodule update --init --recursive` چلائیں، پھر `pnpm install`۔
3. UI کے کام کے لیے `pnpm dev` یا ڈیسک ٹاپ ایپ کے لیے `pnpm dev:app` چلائیں۔ PR کھولنے سے پہلے `pnpm typecheck`، `pnpm format:check` اور `cargo check --manifest-path Cargo.toml` چلائیں۔

مزید [سیٹ اپ کرنا](https://tinyhumans.gitbook.io/openhuman/developing/getting-set-up)، [`AGENTS.md`](../AGENTS.md) اور [کریٹس کا جائزہ](../crates/README.md) میں ہے۔ OpenHuman کے کئی حصے [`vendor/`](../vendor) کے تحت اپنے الگ ریپوز میں ہیں، اور وہ بھی شراکت کا خیر مقدم کرتے ہیں۔

شراکت داروں کو مفت مرچ اور [Discord](https://guild.tinyhumans.ai/) پر خصوصی رسائی ملتی ہے۔

## اسٹار ہسٹری

</div>

<div dir="ltr">

<p align="center">
 <a href="https://www.star-history.com/#tinyhumansai/openhuman&type=date&legend=top-left">
 <picture>
 <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=tinyhumansai/openhuman&type=date&theme=dark&legend=top-left" />
 <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/svg?repos=tinyhumansai/openhuman&type=date&legend=top-left" />
 <img alt="Star History Chart" src="https://api.star-history.com/svg?repos=tinyhumansai/openhuman&type=date&legend=top-left" />
 </picture>
 </a>
</p>

</div>

<div dir="rtl" lang="ur">

## شراکت دار

</div>

<div dir="ltr">

<a href="https://github.com/tinyhumansai/openhuman/graphs/contributors">
 <img src="https://contrib.rocks/image?repo=tinyhumansai/openhuman" alt="OpenHuman شراکت دار" />
</a>

</div>
