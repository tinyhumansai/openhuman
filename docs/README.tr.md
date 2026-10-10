<h1 align="center">OpenHuman</h1>

<p align="center">
 <strong>En hızlı, en ucuz ve en verimli açık kaynak ajan altyapısı. 10 dolarlık bir VPS üzerinde 500'den fazla ajan çalıştırın.</strong><br/>
 İnsanlar için bir masaüstü uygulaması. Geliştiriciler için bir Rust kütüphanesi.
</p>

<p align="center">
 <img src="https://img.shields.io/badge/status-early%20beta-orange" alt="Early Beta" />
 <a href="https://github.com/tinyhumansai/openhuman/releases/latest"><img src="https://img.shields.io/github/v/release/tinyhumansai/openhuman?label=latest" alt="Latest Release" /></a>
 <a href="https://github.com/tinyhumansai/openhuman/stargazers"><img src="https://img.shields.io/github/stars/tinyhumansai/openhuman?style=flat" alt="GitHub Stars" /></a>
 <a href="../LICENSE"><img src="https://img.shields.io/github/license/tinyhumansai/openhuman" alt="License" /></a>
 <a href="https://github.com/tinyhumansai/openhuman-benchmarks"><img src="https://img.shields.io/badge/benchmarks-public-brightgreen" alt="Public benchmarks" /></a>
</p>

<p align="center">
 <a href="https://tinyhumans.gitbook.io/openhuman/">Dokümantasyon</a> ·
 <a href="https://tinyhumans.gitbook.io/openhuman/developing/quickstart">Rust hızlı başlangıç</a> ·
 <a href="https://tinyhumansai.github.io/openhuman-benchmarks/">Kıyaslamalar</a> ·
 <a href="https://github.com/tinyhumansai/openhuman/discussions">Tartışmalar</a> ·
 <a href="https://guild.tinyhumans.ai/">Discord</a> ·
 <a href="https://www.reddit.com/r/tinyhumansai/">Reddit</a> ·
 <a href="https://x.com/intent/follow?screen_name=tinyhumansai">X</a> ·
 <a href="https://x.com/intent/follow?screen_name=senamakel">@senamakel (yaratıcı)</a>
</p>

<p align="center">
 <img src="./demo.gif" alt="OpenHuman masaüstü uygulamasının tanıtım gösterimi" />
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
  🇺🇸 <a href="../README.md">English</a> | 🇸🇦 <a href="./README.ar.md">العربية</a> | 🇨🇳 <a href="./README.zh-CN.md">简体中文</a> | 🇯🇵 <a href="./README.ja-JP.md">日本語</a> | 🇰🇷 <a href="./README.ko.md">한국어</a> | 🇩🇪 <a href="./README.de.md">Deutsch</a> | 🇹🇷 <strong>Türkçe</strong> | 🇵🇰 <a href="./README.ur-pk.md">اردو</a>
</p>

> [!NOTE]
> 🎉 Yayınlandıktan sonraki bir hafta içinde OpenHuman, dokuz gün üst üste **GitHub'da bir numaralı trend depo** oldu.

> **Erken beta.** OpenHuman aktif olarak geliştirilmektedir, bu yüzden bazı pürüzler görebilirsiniz.

---

## Kurulum

En kolay yol, masaüstü uygulamasını [tinyhumans.ai/openhuman](https://tinyhumans.ai/openhuman?utm_source=github&utm_medium=readme) adresinden ya da [son sürümden](https://github.com/tinyhumansai/openhuman/releases/latest) indirmektir. macOS için `.dmg`, Windows için `.msi` veya `.exe`, Linux için `.deb` veya `.AppImage` dosyası bulunur.

Terminali mi tercih ediyorsunuz? Kurulum betiği sisteminize uygun paketi seçer, sağlama toplamını denetler ve kurar:

```bash
# macOS and Linux
curl -fsSL https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.sh | bash
```

```powershell
# Windows (PowerShell)
irm https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.ps1 | iex
```

macOS ve Linux betiğinin ne yapacağını önceden görmek için komutun sonuna `bash -s -- --dry-run` ekleyin. Diğer seçenekler ve sorun giderme [INSTALL.md](../INSTALL.md) dosyasında.

---

## Neden OpenHuman?

Çoğu ajan altyapısı her ajan için ağır bir süreç çalıştırır ve her çağrıda büyük bir istemi yeniden gönderir. OpenHuman aynı işi çok daha azıyla yapar. Büyük ajan filoları için geliştirilmiş, özellik bakımından zengin tek açık kaynak altyapıdır: 10 dolarlık bir sunucuya 500 ajan sığar.

<table>

<tr>

<td width="50%" valign="top">

<h3>Hızlı</h3>

<p><a href="https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/results/swe-x86-1/summary.md">Kıyaslama sonuçları</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/performance">Soğuk başlangıç rakamları</a></p>

<p>Kodlama görevlerini yaklaşık 20 saniyede bitirir; denediğimiz yedi yapay zeka ajan aracının en hızlısıdır. Saniyenin onda birinde açılır.</p>

</td>

<td width="50%" valign="top">

<h3>Ucuz</h3>

<p><a href="https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/results/swe-x86-1/summary.md">Maliyet ve token verileri</a> · <a href="https://tinyhumans.gitbook.io/openhuman/features/token-compression">Sıkıştırma nasıl çalışır</a></p>

<p>Tipik bir ajan aracından 2,6 kat daha az token kullanır ve testimizde en düşük toplam faturayı çıkardı.</p>

</td>

</tr>

<tr>

<td width="50%" valign="top">

<h3>Ölçekte verimli</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/performance">Filo ölçümleri</a> · <a href="https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/profile/docs/library-benchmarking.md">Yöntem</a></p>

<p>Tipik bir ajan aracından yaklaşık 8 kat daha az bellek ve işlemci kullanır. 10 dolarlık bir sunucuda 500'den fazla ajan çalıştırın.</p>

</td>

<td width="50%" valign="top">

<h3>Geliştiriciler için tasarlandı</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/quickstart">Rust hızlı başlangıç</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/embed">Gömme rehberi</a> · <a href="../crates/openhuman-embed/examples">Örnekler</a></p>

<p>Bir Rust kütüphanesi olarak kullanın: bir ajanı herhangi bir işlev gibi çağırın ya da tüm bir filoyu tek bir küçük sunucudan çalıştırın.</p>

</td>

</tr>

</table>


<p align="center">
 <img src="./oh-v-hermes.gif" alt="Hermes vs OpenHuman" />
</p>

<p align="center"><em>Aynı istem, yan yana: "Bana ajan altyapıları hakkında anime tarzında bir hikâye yaz, bunu bir HTML sayfasına yaz ve benim için aç."<br/>OpenHuman 20 saniyede bitirdi; 15 bin token kullandı ve 0,0054 dolar tuttu. Hermes 9 dakika 40 saniye sürdü; 37 bin token kullandı ve 0,0082 dolar tuttu.</em></p>

---

## Başlıca yenilikler

Çoğu ajan altyapısı basit bir döngüdür: her şeyi modele gönder, bekle, tekrarla. Bu tek bir ajan için işe yarar, ama hızla yavaşlar ve pahalılaşır; yüzlerce ajan çalıştırdığınızda ise çöker.

OpenHuman en pahalıya mal olan kısımları yeniden düşünür: yapay zekanın ne kadar metin okuması gerektiği, doğru aracı nasıl bulduğu, özelliklerin nasıl yüklendiği ve her şeyin ne kadar makine gücü istediği. Aşağıdaki altı fikir, yukarıdaki hız ve tasarrufun kaynağıdır. Ayrıntı isterseniz her kart dokümantasyona bağlanır.

<table>

<tr>

<td width="50%" valign="top">

<h3>RLM token sıkıştırma</h3>

<p><a href="https://arxiv.org/abs/2512.24601">RLM makalesi</a> · <a href="https://tinyhumans.gitbook.io/openhuman/features/token-compression">Nasıl çalışır</a></p>

<p><a href="https://arxiv.org/abs/2512.24601">Özyinelemeli Dil Modelleri</a> üzerine kuruludur. Büyük araç sonuçları, yapay zeka okumadan önce sıkıştırılır. Çok büyük olanlarda yapay zeka, hepsini okumak yerine içinde arama yapabileceği bir tutamaç alır. Hiçbir şey atılmaz.</p>

</td>

<td width="50%" valign="top">

<h3>Jev: anında ve doğru araç arama</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/jev">Jev</a> · <a href="./plans/jev-tool-search-baseline.md">Ölçümler</a></p>

<p>Jev, 1.215 araç arasından doğrusunu bulan küçük bir modeldir. Doğru araç, zamanın %86,8'inde ilk seçimleri arasındadır; anahtar kelime aramasında bu oran %70,5'tir.</p>

</td>

</tr>

<tr>

<td width="50%" valign="top">

<h3>Birleşik Rust veri yolu</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/loadable-modules">Eklentiler nasıl çalışır</a></p>

<p>Arama, belgeler ya da ses gibi her özellik tek bir Rust veri yoluna takılır; bu fikir <a href="https://www.freedesktop.org/wiki/Software/dbus/">Linux sistem veri yolundan</a> ödünç alınmıştır. Bir özellik yalnızca gerektiğinde yüklenir ve biri takılırsa diğerleri çalışmaya devam eder.</p>

</td>

<td width="50%" valign="top">

<h3>Derinlemesine entegre bellek</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/features/memory">Bellek nasıl çalışır</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/engines">Bellek motorları</a></p>

<p>Bellek yerleşik gelir. Her turdan önce OpenHuman, bir token bütçesi içinde yalnızca önemli olanı seçer ve kaynaklarıyla birlikte yapay zekaya verir. Kutudan çıktığı gibi çalışır ve bir ayarla farklı bir bellek motoru kullanabilirsiniz.</p>

</td>

</tr>

<tr>

<td width="50%" valign="top">

<h3>Anında tarayıcı ve masaüstü kontrolü</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/features/native-tools/browser-and-computer">Tarayıcı ve bilgisayar kontrolü</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/jev">Jev</a></p>

<p>Ajan gerçek bir tarayıcıyı ve masaüstü uygulamalarınızı kullanır. Jev her tıklamayı ekrandaki düğmelerden seçer, ekran görüntüsü kullanmaz. Herhangi bir ödemeden önce durur.</p>

</td>

<td width="50%" valign="top">

<h3>Programlanabilir bir Rust çekirdeği</h3>

<p><a href="https://tinyhumans.gitbook.io/openhuman/developing/quickstart">Rust hızlı başlangıç</a> · <a href="https://tinyhumans.gitbook.io/openhuman/developing/performance">Performans</a></p>

<p>Tüm altyapı tek bir süreçte derlenmiş Rust'tır; bu yüzden saniyenin onda birinde açılır ve hafif kalır: kodlama görevlerimizde en yüksek noktada 68 MB. Aynı çekirdek bir kütüphanedir. Kendi Rust kodunuzdan bir ajan çağırın ya da yüzlercesini tek bir küçük sunucuda yan yana çalıştırın.</p>

</td>

</tr>

</table>

---

## Kıyaslamalar

<p align="center">
 <picture>
  <source media="(prefers-color-scheme: dark)" srcset="../gitbooks/.gitbook/assets/benchmarks/swe-x86-1-dark.png" />
  <source media="(prefers-color-scheme: light)" srcset="../gitbooks/.gitbook/assets/benchmarks/swe-x86-1.png" />
  <img alt="SWE-bench Verified run swe-x86-1: OpenHuman against six other harnesses on ten panels" src="../gitbooks/.gitbook/assets/benchmarks/swe-x86-1.png" />
 </picture>
</p>

Yedi altyapıyı aynı SWE-bench görevlerinde, aynı model, API anahtarı ve kapsayıcıyla çalıştırdık ve her çağrıyı ölçtük. Kurulum ve sonuçlar [openhuman-benchmarks](https://github.com/tinyhumansai/openhuman-benchmarks) içinde herkese açıktır, böylece herkes yeniden çalıştırabilir. Son çalıştırma on görevli [`swe-x86-1`](https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/results/swe-x86-1/summary.md) çalıştırmasıdır.

OpenHuman görevlerini medyan sürenin yarısından azında bitirdi; 2,6 kat daha az token ve sekizde bir bellek ile işlemci kullandı. Diğerleri 134 ile 212 arasında model çağrısı yaparken o 114 çağrı yaptı, ve diğerlerinin tüm çalıştırması 0,08 ile 0,35 dolar tutarken onunki 0,05 dolar tuttu.

Her adımda daha az iş yaparak kazanır. Çekirdek, Node ya da Python değil, tek süreçte derlenmiş Rust'tır; bu yüzden az bellek kullanır ve model çağrıları arasında neredeyse hiç işlemci harcamaz. Yedi aracın en küçük istemini gönderir: 20 aracı dahil 4,6 bin token. Böylece her çağrı daha ucuz olur ve daha hızlı döner (1,65 sn medyan, bu da en hızlısı). Ayrıca bir yanıta varmak için daha az çağrı gerekir ve tasarrufun büyük kısmı buradan gelir.

| Çözülen görev başına | OpenHuman (medyana göre) | Diğer altısının medyanı | Diğer altısının en iyisi |
| --- | --- | --- | --- |
| Duvar saati süresi (p50) | **19,8 sn** (2,3 kat daha hızlı) | 46,5 sn | 28,4 sn (OpenCode) |
| Token | **167 bin** (2,6 kat daha az) | 435 bin | 370 bin (Codex) |
| Maliyet | **$0.0077** (-%36) | $0.012 | $0.0077 (OpenCode, berabere) |
| En yüksek bellek | **68 MB** (7,7 kat daha az) | 523 MB | 123 MB (Codex) |
| İşlemci süresi | **1,3 sn** (8 kat daha az) | 10,4 sn | 2,9 sn (DeepSeek Harness) |
| Sistem istemi | **4,6 bin token** (-%40) | 7,7 bin | 6,2 bin (DeepSeek Harness) |

---

## Kullanıcılar için

Claude Code, Codex, OpenClaw ya da Hermes kullanıyorsanız fikirleri zaten biliyorsunuzdur: araçlı bir ajan döngüsü, MCP, yetenekler, BYOK modeller ve bellek. OpenHuman hepsini bir masaüstü uygulamasında, bir terminal uygulamasında ya da başsız bir sunucuda sunar.

| Yetenek | OpenHuman'ın sunduğu |
| --- | --- |
| Yapılandırılabilir bellek | [Dosyalarınız, depolarınız, akışlarınız ve uygulamalarınız üzerinde yerleşik bellek](https://tinyhumans.gitbook.io/openhuman/features/memory); seçtiğiniz bellek motoruyla, her turdan önce kaynaklarıyla birlikte hatırlanır |
| Sesli ajanlar | Cümlenin ortasında sözünü kesebileceğiniz [canlı bir sesli ajan](https://tinyhumans.gitbook.io/openhuman/features/native-tools/voice), ayrıca dikte ve sesli yanıtlar |
| Bilgisayar ve tarayıcı kontrolü | [Gerçek bir Chrome tarayıcısını ve masaüstü uygulamalarınızı yönetir](https://tinyhumans.gitbook.io/openhuman/features/native-tools/browser-and-computer); geri alamayacağı bir şeyden önce sorar |
| Arama | [Arama motorları ve arama ajanları](https://tinyhumans.gitbook.io/openhuman/features/native-tools/web-search): Exa ve Gemini dahil, Brave, Tavily, Parallel ve daha fazlası kendi anahtarınızla; kaynaklı yanıtlar ve Deep Research |
| Araçlar | [Yerel araçlar](https://tinyhumans.gitbook.io/openhuman/features/native-tools): kabuk ve kodlayıcı, kazıyıcı, belgeler, görsel ve video üretimi, cron |
| MCP ve yetenekler | [MCP sunucuları ve yetenek paketleri](https://tinyhumans.gitbook.io/openhuman/features/integrations/mcp-and-skills) |
| OAuth entegrasyonları | [Composio üzerinden 119 uygulama](https://tinyhumans.gitbook.io/openhuman/features/integrations), bir şey olduğunda ajanı başlatan [tetikleyicilerle](https://tinyhumans.gitbook.io/openhuman/features/integrations/triggers) |
| Modeller | [Yerel modeller (Ollama, LM Studio, MLX) ve 26 sağlayıcı için BYOK](https://tinyhumans.gitbook.io/openhuman/features/model-routing/local-and-byok-models), [otomatik model yönlendirmeyle](https://tinyhumans.gitbook.io/openhuman/features/model-routing) |
| Kanallar | Ajan ön yüzü olarak [14 mesajlaşma kanalı](https://tinyhumans.gitbook.io/openhuman/features/channels): Telegram, Discord, iMessage, e-posta ve daha fazlası |
| İş akışları | [Dayanıklı iş akışı grafikleri](https://tinyhumans.gitbook.io/openhuman/features/workflows): cron, olay veya elle tetikleyiciler, onay adımları, duraklatmadan sonra devam etme |
| Güvenlik | [Onay kapısı](https://tinyhumans.gitbook.io/openhuman/features/approval-gate), [korumalı çalıştırma](https://tinyhumans.gitbook.io/openhuman/features/privacy-and-security) (işletim sistemi hapsi veya Docker), yalnızca yerel çalıştırmalar için [Gizlilik Modu](https://tinyhumans.gitbook.io/openhuman/features/privacy-mode), gizli bilgiler [işletim sistemi anahtarlığında](https://tinyhumans.gitbook.io/openhuman/features/os-keyring-and-secret-storage) |
| Kullanım takibi | [Çağrı başına maliyet ve token kullanımı](https://tinyhumans.gitbook.io/openhuman/features/billing-and-usage), ayrıca yeniden oynatılabilir çalışma günlükleri |

[Başlarken](https://tinyhumans.gitbook.io/openhuman/overview/getting-started) bölümü veya [rehberlerle](https://tinyhumans.gitbook.io/openhuman/guides) başlayın.

---

## Geliştiriciler için

OpenHuman, kütüphane öncelikli bir altyapıdır. Rust projenize ekleyin ve bir ajanı herhangi bir işlev gibi çağırın; çalıştırılacak bir yan süreç ya da arka plan hizmeti yoktur. Tek bir çalışma zamanı, her biri kendi modeli, araçları, belleği ve korumalı alanı olan yüzlerce ajanı barındırabilir. 10 dolarlık bir VPS'e 500 tanesi sığar, bu yüzden müşteri başına bir ajan sunan bir ürün küme kurmadan başlayabilir.

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

Sırada: [Rust hızlı başlangıç](https://tinyhumans.gitbook.io/openhuman/developing/quickstart), [gömme rehberi](https://tinyhumans.gitbook.io/openhuman/developing/embed) ve [geliştirici dokümanları](https://tinyhumans.gitbook.io/openhuman/developing).

---

## Diğerleriyle karşılaştırma

İşte OpenHuman, çoğu insanın zaten kullandığı altyapıların yanında. İlk dört satır [herkese açık kıyaslamamızdan](https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/results/swe-x86-1/summary.md) gelir: her altyapı için aynı model ve kapsayıcıyla on SWE-bench kodlama görevi. Her sayı çözülen görev başınadır. Geri kalanı özellikleri karşılaştırır.

Kısacası diğerleri, tek bir ajanla çalışan tek bir kişi için harikadır. OpenHuman bunu da yapar; ayrıca kütüphane olarak gömebileceğiniz ve bir filoya ölçekleyebileceğiniz açık kaynak seçenektir. Ürünler hızla değişir, bu yüzden karar vermeden önce her projeyi kontrol edin.

|                         | Claude Code          | Codex                | OpenClaw             | Hermes Agent         | OpenHuman                                |
| ----------------------- | -------------------- | -------------------- | -------------------- | -------------------- | ---------------------------------------- |
| **Medyan görev süresi**    | 37,8 sn               | 36,1 sn               | 62 sn                 | 58,5 sn               | 🚀 19,8 sn                                 |
| **Görev başına ort. token** | 428 bin                 | 370 bin                | 442 bin                | 482 bin                | 🚀 167 bin                                   |
| **Görev başına en yüksek bellek** | 231 MB               | 123 MB               | 1,57 GB              | 816 MB               | 🚀 68 MB                                  |
| **Görev başına ort. maliyet**   | $0.04                | $0.02                | $0.01                | $0.0082              | 🚀 $0.0077                                |
| **Açık kaynak**         | 🚫 Tescilli       | ✅ Apache-2.0        | ✅ MIT               | ✅ MIT               | ✅ GPL-3.0                               |
| **Ajan filoları**        | ⚠️ Ajan başına süreç | ⚠️ Ajan başına süreç | ⚠️ Ajan başına süreç | ⚠️ Ajan başına süreç | 🚀 10 dolarlık VPS'te 500 ajan               |
| **Gömülebilir kütüphane**  | ⚠️ CLI üzerinde SDK    | ⚠️ CLI üzerinde SDK    | 🚫 Yok              | ⚠️ Python paketi    | 🚀 Türlendirilmiş Rust API'si                        |
| **Bellek**              | ⚠️ Bellek dosyaları      | ⚠️ Bellek dosyaları      | ⚠️ Eklentiye bağımlı    | ✅ Kendi kendine öğrenen     | 🚀 Her turda kaynaklarıyla hatırlanır   |
| **Entegrasyonlar**        | ✅ MCP               | ✅ MCP               | ⚠️ Kendin getir               | ⚠️ Kendin getir               | 🚀 119 OAuth uygulaması, MCP, yetenekler           |
| **Mesajlaşma kanalları**  | 🚫 Yok              | 🚫 Yok              | ✅ Çok sayıda            | ✅ Birkaç           | ✅ E-posta dahil 14                   |
| **Tarayıcı ve masaüstü** | ⚠️ MCP ile tarayıcı   | ⚠️ MCP ile tarayıcı   | ✅ Tarayıcı           | ✅ Tarayıcı           | ✅ Tarayıcı ve masaüstü uygulamaları              |
| **İş akışları**           | 🚫 Yok              | 🚫 Yok              | ⚠️ Betikler           | ⚠️ Betikler           | 🚀 Görsel, ajan tarafından taslaklanan                 |
| **Model seçimi**        | ⚠️ Anthropic modelleri  | ⚠️ Önce OpenAI      | ✅ Herhangi biri               | ✅ Herhangi biri               | ✅ Herhangi biri, yerleşik yönlendirmeyle            |

---

## Katkıda bulunma

[`CONTRIBUTING.md`](../CONTRIBUTING.md) dosyasını okuyun ya da bir yapay zeka kodlama ajanının sizi yönlendirmesi için [bu istemi](./CONTRIBUTING-BEGINNERS.md#optional--let-an-ai-coding-agent-guide-you) kullanın.

1. Git, Node.js 24+, pnpm 10.10.0, Rust 1.96.1 (`rustfmt` ve `clippy` ile), CMake, Ninja, ripgrep ve platformunuzun masaüstü derleme önkoşullarını kurun.
2. Depoyu çatallayın ve klonlayın. `git submodule update --init --recursive` komutunu, ardından `pnpm install` komutunu çalıştırın.
3. Arayüz çalışması için `pnpm dev`, masaüstü uygulaması için `pnpm dev:app` çalıştırın. PR açmadan önce `pnpm typecheck`, `pnpm format:check` ve `cargo check --manifest-path Cargo.toml` çalıştırın.

Daha fazlası [Kurulum](https://tinyhumans.gitbook.io/openhuman/developing/getting-set-up), [`AGENTS.md`](../AGENTS.md) ve [crate'lere genel bakış](../crates/README.md) bölümlerinde. OpenHuman'ın birçok parçası [`vendor/`](../vendor) altında kendi depolarında yaşar ve onlar da katkıları memnuniyetle karşılar.

Katkıda bulunanlar ücretsiz hediyeler ve [Discord](https://guild.tinyhumans.ai/) üzerinde özel erişim kazanır.

## Yıldız geçmişi

<p align="center">
 <a href="https://www.star-history.com/#tinyhumansai/openhuman&type=date&legend=top-left">
 <picture>
 <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=tinyhumansai/openhuman&type=date&theme=dark&legend=top-left" />
 <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/svg?repos=tinyhumansai/openhuman&type=date&legend=top-left" />
 <img alt="Star History Chart" src="https://api.star-history.com/svg?repos=tinyhumansai/openhuman&type=date&legend=top-left" />
 </picture>
 </a>
</p>

## Katkıda bulunanlar

<a href="https://github.com/tinyhumansai/openhuman/graphs/contributors">
 <img src="https://contrib.rocks/image?repo=tinyhumansai/openhuman" alt="OpenHuman contributors" />
</a>
