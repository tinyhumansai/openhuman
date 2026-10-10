<div dir="ltr">

<h1 align="center">OpenHuman</h1>

</div>

<div dir="rtl" lang="ar">

<p align="center">
 <strong>أسرع منظومة وكلاء مفتوحة المصدر وأرخصها وأكثرها كفاءة. شغّل أكثر من 500 وكيل على خادم VPS بسعر 10 دولارات.</strong><br/>
 تطبيق سطح مكتب للناس. ومكتبة Rust للمطورين.
</p>

</div>

<div dir="ltr">

<p align="center">
 <img src="https://img.shields.io/badge/status-early%20beta-orange" alt="Early Beta" />
 <a href="https://github.com/tinyhumansai/openhuman/releases/latest"><img src="https://img.shields.io/github/v/release/tinyhumansai/openhuman?label=latest" alt="Latest Release" /></a>
 <a href="https://github.com/tinyhumansai/openhuman/stargazers"><img src="https://img.shields.io/github/stars/tinyhumansai/openhuman?style=flat" alt="GitHub Stars" /></a>
 <a href="../LICENSE"><img src="https://img.shields.io/github/license/tinyhumansai/openhuman" alt="License" /></a>
 <a href="https://github.com/tinyhumansai/openhuman-benchmarks"><img src="https://img.shields.io/badge/benchmarks-public-brightgreen" alt="Public benchmarks" /></a>
</p>

<p align="center">
 <a href="https://tinyhumans.gitbook.io/openhuman/">المستندات</a> ·
 <a href="https://tinyhumans.gitbook.io/openhuman/developing/quickstart">البدء السريع مع Rust</a> ·
 <a href="https://tinyhumansai.github.io/openhuman-benchmarks/">اختبارات الأداء</a> ·
 <a href="https://github.com/tinyhumansai/openhuman/discussions">النقاشات</a> ·
 <a href="https://guild.tinyhumans.ai/">Discord</a> ·
 <a href="https://www.reddit.com/r/tinyhumansai/">Reddit</a> ·
 <a href="https://x.com/intent/follow?screen_name=tinyhumansai">X</a> ·
 <a href="https://x.com/intent/follow?screen_name=senamakel">@senamakel (المؤسس)</a>
</p>

<p align="center">
 <img src="./demo.gif" alt="عرض توضيحي لتطبيق OpenHuman لسطح المكتب" />
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
  🇺🇸 <a href="../README.md">English</a> | 🇸🇦 <a href="./README.ar.md"><strong>العربية</strong></a> | 🇨🇳 <a href="./README.zh-CN.md">简体中文</a> | 🇯🇵 <a href="./README.ja-JP.md">日本語</a> | 🇰🇷 <a href="./README.ko.md">한국어</a> | 🇩🇪 <a href="./README.de.md">Deutsch</a> | 🇹🇷 <a href="./README.tr.md">Türkçe</a> | 🇵🇰 <a href="./README.ur-pk.md">اردو</a>
</p>

</div>

<div dir="rtl" lang="ar">

> [!NOTE]
> 🎉 خلال أسبوع واحد من الإطلاق، أصبح OpenHuman **المستودع الأول في قائمة الأكثر رواجاً على GitHub** لتسعة أيام متتالية.

> **إصدار تجريبي أولي.** يتطور OpenHuman بنشاط، لذا توقّع بعض الجوانب غير المكتملة.

---

## التثبيت

أسهل طريقة هي تنزيل تطبيق سطح المكتب من [tinyhumans.ai/openhuman](https://tinyhumans.ai/openhuman?utm_source=github&utm_medium=readme) أو من [أحدث إصدار](https://github.com/tinyhumansai/openhuman/releases/latest). يتوفر ملف `.dmg` لنظام macOS، وملف `.msi` أو `.exe` لنظام Windows، وملف `.deb` أو `.AppImage` لنظام Linux.

تفضّل الطرفية؟ يختار سكربت التثبيت الحزمة المناسبة لنظامك، ويتحقق من مجموعها الاختباري (checksum)، ثم يثبّتها:

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

<div dir="rtl" lang="ar">

لمعاينة ما سيفعله السكربت على macOS وLinux، أنهِ الأمر بـ `bash -s -- --dry-run`. تجد خيارات أخرى وحلولاً للمشكلات في [INSTALL.md](../INSTALL.md).

---

## لماذا OpenHuman؟

تشغّل معظم المنظومات عملية ثقيلة واحدة لكل وكيل، وتعيد إرسال موجّه (prompt) كبير مع كل استدعاء. يؤدي OpenHuman العمل نفسه بموارد أقل بكثير. وهو المنظومة المفتوحة المصدر الغنية بالميزات الوحيدة المصممة لأساطيل كبيرة من الوكلاء: يتسع 500 وكيل منها على خادم بسعر 10 دولارات.

</div>

<div dir="ltr">

<table>

<tr>

<td width="50%" valign="top" dir="rtl">

<h3>سريع</h3>

<p><a href="https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/results/swe-x86-1/summary.md">نتائج الاختبارات</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/performance">أرقام بدء التشغيل البارد</a></p>

<p>ينجز مهام البرمجة في نحو 20 ثانية، وهو الأسرع بين سبع أدوات وكلاء ذكاء اصطناعي اختبرناها. ويبدأ العمل خلال عُشر ثانية.</p>

</td>

<td width="50%" valign="top" dir="rtl">

<h3>رخيص</h3>

<p><a href="https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/results/swe-x86-1/summary.md">بيانات التكلفة والرموز</a> · <a href="https://tinyhumans.gitbook.io/openhuman/features/token-compression">كيف يعمل الضغط</a></p>

<p>يستخدم رموزاً (tokens) أقل بـ 2.6x من أداة الوكلاء المعتادة، وكانت فاتورته الإجمالية الأقل في اختبارنا.</p>

</td>

</tr>

<tr>

<td width="50%" valign="top" dir="rtl">

<h3>كفاءة عند التوسع</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/performance">قياسات الأساطيل</a> · <a href="https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/profile/docs/library-benchmarking.md">المنهجية</a></p>

<p>يستخدم ذاكرة ومعالجاً أقل بنحو 8x من أداة الوكلاء المعتادة. شغّل أكثر من 500 وكيل على خادم بسعر 10 دولارات.</p>

</td>

<td width="50%" valign="top" dir="rtl">

<h3>مصمم للمطورين</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/quickstart">البدء السريع مع Rust</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/embedding">دليل التضمين</a> · <a href="../crates/openhuman-embed/examples">أمثلة</a></p>

<p>استخدمه كمكتبة Rust: استدعِ وكيلاً كأي دالة أخرى، أو شغّل أسطولاً كاملاً من خادم صغير واحد.</p>

</td>

</tr>

</table>


<p align="center">
 <img src="./oh-v-hermes.gif" alt="Hermes vs OpenHuman" />
</p>

</div>

<div dir="rtl" lang="ar">

<p align="center"><em>الموجّه نفسه، جنباً إلى جنب: "Write me a story about agent harnesses in the form of an anime story, write it into an HTML page and open it for me."<br/>أنهى OpenHuman المهمة في 20 ثانية، باستخدام 15k رمز وبتكلفة 0.0054 دولار. أما Hermes فاستغرق 9 دقائق و40 ثانية، باستخدام 37k رمز وبتكلفة 0.0082 دولار.</em></p>

---

## أبرز الابتكارات

معظم المنظومات حلقة بسيطة: أرسل كل شيء إلى النموذج، وانتظر، ثم كرّر. هذا يصلح لوكيل واحد، لكنه يصبح بطيئاً ومكلفاً بسرعة، وينهار عند تشغيل المئات.

يعيد OpenHuman التفكير في الأجزاء الأعلى تكلفة: كم من النص يقرأ الذكاء الاصطناعي، وكيف يجد الأداة المناسبة، وكيف تُحمَّل الميزات، وكم من الجهاز يحتاج النظام كله. الأفكار الست أدناه هي مصدر السرعة والتوفير المذكورين أعلاه. تشير كل بطاقة إلى المستندات إن أردت التفاصيل.

</div>

<div dir="ltr">

<table>

<tr>

<td width="50%" valign="top" dir="rtl">

<h3>ضغط الرموز بتقنية RLM</h3>

<p><a href="https://arxiv.org/abs/2512.24601">ورقة RLM</a> · <a href="https://tinyhumans.gitbook.io/openhuman/features/token-compression">كيف يعمل</a></p>

<p>مبني على <a href="https://arxiv.org/abs/2512.24601">النماذج اللغوية العودية (Recursive Language Models)</a>. تُضغط نتائج الأدوات الكبيرة قبل أن يقرأها الذكاء الاصطناعي. وفي النتائج الكبيرة جداً، يحصل الذكاء الاصطناعي على مقبض يستطيع البحث فيه بدل قراءتها كلها. لا يُرمى شيء.</p>

</td>

<td width="50%" valign="top" dir="rtl">

<h3>Jev: بحث فوري ودقيق عن الأدوات</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/jev">Jev</a> · <a href="./plans/jev-tool-search-baseline.md">القياسات</a></p>

<p>Jev نموذج صغير يجد الأداة المناسبة من بين 1,215 أداة. تكون الأداة الصحيحة ضمن أفضل اختياراته بنسبة 86.8% من الحالات، مقابل 70.5% للبحث بالكلمات المفتاحية.</p>

</td>

</tr>

<tr>

<td width="50%" valign="top" dir="rtl">

<h3>ناقل Rust موحّد</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/loadable-modules">كيف تعمل الإضافات</a></p>

<p>كل ميزة، مثل البحث أو المستندات أو الصوت، تتصل بناقل Rust واحد، وهي فكرة مستعارة من <a href="https://www.freedesktop.org/wiki/Software/dbus/">ناقل نظام Linux</a>. لا تُحمَّل الميزة إلا عند الحاجة، وإذا تعطلت إحداها تواصل البقية عملها.</p>

</td>

<td width="50%" valign="top" dir="rtl">

<h3>ذاكرة متكاملة بعمق</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/features/memory">كيف تعمل الذاكرة</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/engines">محركات الذاكرة</a></p>

<p>الذاكرة مدمجة. قبل كل دور، يختار OpenHuman ما يهم فقط ضمن ميزانية من الرموز، ويقدمه للذكاء الاصطناعي مع مصادره. تعمل مباشرة دون إعداد، ويمكنك استبدال محرك ذاكرة آخر بإعداد واحد.</p>

</td>

</tr>

<tr>

<td width="50%" valign="top" dir="rtl">

<h3>تحكم فوري بالمتصفح وسطح المكتب</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/features/native-tools/browser-and-computer">التحكم بالمتصفح والحاسوب</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/jev">Jev</a></p>

<p>يستخدم الوكيل متصفحاً حقيقياً وتطبيقات سطح مكتبك. يختار Jev كل نقرة من الأزرار الظاهرة على الشاشة، دون لقطات شاشة. ويتوقف قبل أي عملية دفع.</p>

</td>

<td width="50%" valign="top" dir="rtl">

<h3>نواة Rust قابلة للبرمجة</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/quickstart">البدء السريع مع Rust</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/performance">الأداء</a></p>

<p>المنظومة كلها Rust مترجَم في عملية واحدة، فتبدأ خلال عُشر ثانية وتبقى خفيفة: 68 MB في الذروة في مهام البرمجة لدينا. والنواة نفسها مكتبة. استدعِ وكيلاً من شيفرة Rust الخاصة بك، أو شغّل المئات منهم جنباً إلى جنب على خادم صغير واحد.</p>

</td>

</tr>

</table>

---

</div>

<div dir="rtl" lang="ar">

## اختبارات الأداء

</div>

<div dir="ltr">

<p align="center">
 <picture>
  <source media="(prefers-color-scheme: dark)" srcset="../gitbooks/.gitbook/assets/benchmarks/swe-x86-1-dark.png" />
  <source media="(prefers-color-scheme: light)" srcset="../gitbooks/.gitbook/assets/benchmarks/swe-x86-1.png" />
  <img alt="SWE-bench Verified run swe-x86-1: OpenHuman against six other harnesses on ten panels" src="../gitbooks/.gitbook/assets/benchmarks/swe-x86-1.png" />
 </picture>
</p>

</div>

<div dir="rtl" lang="ar">

شغّلنا سبع منظومات على مهام SWE-bench نفسها، بالنموذج ومفتاح API والحاوية نفسها، وقِسنا كل استدعاء. الإعداد والنتائج علنيان في [openhuman-benchmarks](https://github.com/tinyhumansai/openhuman-benchmarks)، فيستطيع أي شخص إعادة تشغيلها. أحدث تشغيل هو [`swe-x86-1`](https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/results/swe-x86-1/summary.md)، وفيه عشر مهام.

أنهى OpenHuman مهامه في أقل من نصف الزمن الوسيط، باستخدام رموز أقل بـ 2.6x وثُمن الذاكرة والمعالج. أجرى 114 استدعاءً للنموذج بينما أجرت الأخرى من 134 إلى 212، وكلّف تشغيله كله 0.05 دولار بينما كلّفت الأخرى من 0.08 إلى 0.35 دولار.

يفوز لأنه يبذل جهداً أقل في كل خطوة. النواة Rust مترجَم في عملية واحدة، وليست Node أو Python، لذا تستهلك ذاكرة قليلة ومعالجاً شبه معدوم بين استدعاءات النموذج. ويرسل أصغر موجّه بين السبعة، 4.6k رمز شاملة أدواته العشرين، فيصبح كل استدعاء أرخص ويعود أسرع (1.65 ثانية وسيطاً، وهي الأسرع أيضاً). كما يحتاج إلى استدعاءات أقل للوصول إلى إجابة، وهنا يأتي معظم التوفير.

| لكل مهمة محلولة | OpenHuman (مقابل الوسيط) | وسيط الستة الآخرين | الأفضل بين الستة الآخرين |
| --- | --- | --- | --- |
| زمن التنفيذ (p50) | **19.8 s** (أسرع بـ 2.3x) | 46.5 s | 28.4 s (OpenCode) |
| الرموز | **167k** (أقل بـ 2.6x) | 435k | 370k (Codex) |
| التكلفة | **$0.0077** (-36%) | $0.012 | $0.0077 (OpenCode، تعادل) |
| ذروة الذاكرة | **68 MB** (أقل بـ 7.7x) | 523 MB | 123 MB (Codex) |
| زمن المعالج | **1.3 s** (أقل بـ 8x) | 10.4 s | 2.9 s (DeepSeek Harness) |
| موجّه النظام | **4.6k رمز** (-40%) | 7.7k | 6.2k (DeepSeek Harness) |

---

## للمستخدمين

إذا كنت تستخدم Claude Code أو Codex أو OpenClaw أو Hermes فأنت تعرف الأفكار: حلقة وكيل مع أدوات، وMCP، والمهارات (skills)، ونماذج BYOK، والذاكرة. يضم OpenHuman كل ذلك، في تطبيق سطح مكتب أو تطبيق طرفية أو خادم بلا واجهة.

| القدرة | ما يقدمه OpenHuman |
| --- | --- |
| ذاكرة قابلة للضبط | [ذاكرة مدمجة فوق ملفاتك ومستودعاتك ومصادرك وتطبيقاتك](https://tinyhumans.gitbook.io/openhuman/features/memory)، تُستدعى قبل كل دور مع مصادرها، على محرك الذاكرة الذي تختاره |
| وكلاء صوتيون | [وكيل صوتي مباشر](https://tinyhumans.gitbook.io/openhuman/features/native-tools/voice) يمكنك مقاطعته في منتصف الجملة، إضافة إلى الإملاء والردود المنطوقة |
| التحكم بالحاسوب والمتصفح | [يقود متصفح Chrome حقيقياً وتطبيقات سطح مكتبك](https://tinyhumans.gitbook.io/openhuman/features/native-tools/browser-and-computer)، ويسأل قبل أي أمر لا يمكن التراجع عنه |
| البحث | [محركات البحث ووكلاء البحث](https://tinyhumans.gitbook.io/openhuman/features/native-tools/web-search): Exa وGemini مضمّنان، وBrave وTavily وParallel وغيرها بمفتاحك الخاص، مع إجابات موثّقة بمصادرها وDeep Research |
| الأدوات | [أدوات أصلية](https://tinyhumans.gitbook.io/openhuman/features/native-tools): الطرفية والبرمجة، وكاشط الويب، والمستندات، وتوليد الصور والفيديو، وcron |
| MCP والمهارات | [خوادم MCP وحزم المهارات](https://tinyhumans.gitbook.io/openhuman/features/integrations/mcp-and-skills) |
| تكاملات OAuth | [119 تطبيقاً عبر Composio](https://tinyhumans.gitbook.io/openhuman/features/integrations)، مع [مشغّلات (triggers)](https://tinyhumans.gitbook.io/openhuman/features/integrations/triggers) تبدأ الوكيل عند حدوث أمر ما |
| النماذج | [نماذج محلية (Ollama وLM Studio وMLX) وBYOK لـ 27 مزوداً](https://tinyhumans.gitbook.io/openhuman/features/model-routing/local-and-byok-models)، مع [توجيه تلقائي للنماذج](https://tinyhumans.gitbook.io/openhuman/features/model-routing) |
| القنوات | [14 قناة مراسلة](https://tinyhumans.gitbook.io/openhuman/features/channels) كواجهات للوكيل: Telegram وDiscord وiMessage والبريد الإلكتروني وغيرها |
| سير العمل | [مخططات سير عمل دائمة](https://tinyhumans.gitbook.io/openhuman/features/workflows): مشغّلات cron أو الأحداث أو اليدوية، وخطوات موافقة، واستئناف بعد التوقف |
| الأمان | [بوابة الموافقة](https://tinyhumans.gitbook.io/openhuman/features/approval-gate)، و[التنفيذ المعزول](https://tinyhumans.gitbook.io/openhuman/features/privacy-and-security) (سجن نظام التشغيل أو Docker)، و[وضع الخصوصية](https://tinyhumans.gitbook.io/openhuman/features/privacy-mode) للتشغيل المحلي فقط، والأسرار في [سلسلة مفاتيح نظام التشغيل](https://tinyhumans.gitbook.io/openhuman/features/os-keyring-and-secret-storage) |
| تتبع الاستخدام | [تكلفة كل استدعاء واستخدام الرموز](https://tinyhumans.gitbook.io/openhuman/features/billing-and-usage)، إضافة إلى سجلات تشغيل قابلة لإعادة التشغيل |

ابدأ من [البدء](https://tinyhumans.gitbook.io/openhuman/overview/getting-started) أو من [الأدلة](https://tinyhumans.gitbook.io/openhuman/guides).

---

## للمطورين

OpenHuman منظومة تضع المكتبة أولاً. أضفه إلى مشروع Rust الخاص بك واستدعِ وكيلاً كأي دالة أخرى، دون خدمة جانبية أو عملية خلفية لتشغيلها. يستطيع وقت تشغيل واحد استضافة مئات الوكلاء، لكل منهم نموذجه وأدواته وذاكرته وبيئته المعزولة. يتسع 500 منهم على خادم VPS بسعر 10 دولارات، فيمكن لمنتج بوكيل لكل عميل أن يبدأ دون عنقود خوادم.

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

<div dir="rtl" lang="ar">

بعد ذلك: [البدء السريع مع Rust](https://tinyhumans.gitbook.io/openhuman/developing/quickstart)، و[دليل التضمين](https://tinyhumans.gitbook.io/openhuman/developing/embedding)، و[مستندات المطورين](https://tinyhumans.gitbook.io/openhuman/developing).

---

## كيف يقارن بغيره؟

هنا نضع OpenHuman بجانب المنظومات التي يستخدمها معظم الناس بالفعل. تأتي الصفوف الأربعة الأولى من [اختبار الأداء العلني](https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/results/swe-x86-1/summary.md): عشر مهام برمجة من SWE-bench، بالنموذج والحاوية نفسيهما لكل منظومة. كل رقم هو لكل مهمة محلولة. وبقية الصفوف تقارن الميزات.

باختصار، الأدوات الأخرى ممتازة لشخص واحد يعمل مع وكيل واحد. OpenHuman يفعل ذلك أيضاً، وهو كذلك الخيار المفتوح المصدر الذي يمكنك تضمينه كمكتبة وتوسيعه إلى أسطول. المنتجات تتغير بسرعة، فتحقق من كل مشروع قبل أن تقرر.

</div>

<div dir="ltr">

|                         | Claude Code          | Codex                | OpenClaw             | Hermes Agent         | OpenHuman                                |
| ----------------------- | -------------------- | -------------------- | -------------------- | -------------------- | ---------------------------------------- |
| **وسيط زمن المهمة**    | 37.8 s               | 36.1 s               | 62 s                 | 58.5 s               | 🚀 19.8 s                                 |
| **متوسط الرموز لكل مهمة** | 428k                 | 370k                 | 442k                 | 482k                 | 🚀 167k                                   |
| **ذروة الذاكرة لكل مهمة** | 231 MB               | 123 MB               | 1.57 GB              | 816 MB               | 🚀 68 MB                                  |
| **متوسط التكلفة لكل مهمة**   | $0.04                | $0.02                | $0.01                | $0.0082              | 🚀 $0.0077                                |
| **مفتوح المصدر**         | 🚫 مملوك       | ✅ Apache-2.0        | ✅ MIT               | ✅ MIT               | ✅ GPL-3.0                               |
| **أساطيل الوكلاء**        | ⚠️ عملية لكل وكيل | ⚠️ عملية لكل وكيل | ⚠️ عملية لكل وكيل | ⚠️ عملية لكل وكيل | 🚀 500 وكيل على VPS بسعر 10 دولارات               |
| **مكتبة قابلة للتضمين**  | ⚠️ SDK فوق CLI    | ⚠️ SDK فوق CLI    | 🚫 لا يوجد              | ⚠️ حزمة Python    | 🚀 واجهة Rust مكتوبة الأنواع                        |
| **الذاكرة**              | ⚠️ ملفات ذاكرة      | ⚠️ ملفات ذاكرة      | ⚠️ تعتمد على الإضافات    | ✅ تعلّم ذاتي     | 🚀 تُستدعى في كل دور، مع المصادر   |
| **التكاملات**        | ✅ MCP               | ✅ MCP               | ⚠️ أحضر بنفسك               | ⚠️ أحضر بنفسك               | 🚀 119 تطبيق OAuth وMCP والمهارات           |
| **قنوات المراسلة**  | 🚫 لا يوجد              | 🚫 لا يوجد              | ✅ كثيرة              | ✅ عدة قنوات           | ✅ 14، منها البريد الإلكتروني                   |
| **المتصفح وسطح المكتب** | ⚠️ المتصفح عبر MCP   | ⚠️ المتصفح عبر MCP   | ✅ المتصفح           | ✅ المتصفح           | ✅ المتصفح وتطبيقات سطح المكتب              |
| **سير العمل**           | 🚫 لا يوجد              | 🚫 لا يوجد              | ⚠️ سكربتات           | ⚠️ سكربتات           | 🚀 مرئي، يصيغه الوكيل                 |
| **اختيار النموذج**        | ⚠️ نماذج Anthropic  | ⚠️ OpenAI أولاً      | ✅ أي نموذج               | ✅ أي نموذج               | ✅ أي نموذج، مع توجيه مدمج            |

</div>

<div dir="rtl" lang="ar">

---

## المساهمة

اقرأ [`CONTRIBUTING.md`](../CONTRIBUTING.md)، أو دع وكيل برمجة بالذكاء الاصطناعي يرشدك عبر [هذا الموجّه](./CONTRIBUTING-BEGINNERS.md#optional--let-an-ai-coding-agent-guide-you).

1. ثبّت Git وNode.js 24+ وpnpm 10.10.0 وRust 1.96.1 (مع `rustfmt` و`clippy`) وCMake وNinja وripgrep، ومتطلبات بناء تطبيقات سطح المكتب لمنصتك.
2. انسخ المستودع (fork) ثم استنسخه. شغّل `git submodule update --init --recursive`، ثم `pnpm install`.
3. شغّل `pnpm dev` لعمل الواجهة أو `pnpm dev:app` لتطبيق سطح المكتب. قبل فتح طلب سحب (PR)، شغّل `pnpm typecheck` و`pnpm format:check` و`cargo check --manifest-path Cargo.toml`.

المزيد في [الإعداد للتطوير](https://tinyhumans.gitbook.io/openhuman/developing/getting-set-up)، و[`AGENTS.md`](../AGENTS.md)، و[نظرة عامة على الحزم (crates)](../crates/README.md). تعيش أجزاء كثيرة من OpenHuman في مستودعاتها الخاصة تحت [`vendor/`](../vendor)، وهي ترحب بالمساهمات أيضاً.

يحصل المساهمون على هدايا مجانية ووصول خاص على [Discord](https://guild.tinyhumans.ai/).

## تاريخ النجوم

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

<div dir="rtl" lang="ar">

## المساهمون

</div>

<div dir="ltr">

<a href="https://github.com/tinyhumansai/openhuman/graphs/contributors">
 <img src="https://contrib.rocks/image?repo=tinyhumansai/openhuman" alt="OpenHuman contributors" />
</a>

</div>
