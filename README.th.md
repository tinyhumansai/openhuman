# OpenHuman (เวอร์ชันภาษาไทยสำหรับ Local Machine)

**ผู้ช่วย AI อัจฉริยะแบบ Local-first — สถาปัตยกรรม React + Vite + Rust Core (JSON-RPC) รองรับภาษาไทย 100% เชื่อมต่อ Ollama และ Obsidian Vault**

---

## 🌟 ฟีเจอร์เด่นในรุ่นนี้ (Thai Edition Highlights)

1. **รองรับภาษาไทยสมบูรณ์แบบ 100%**: แปลภาษาหน้าจอ UI ครบถ้วนทุกเมนู (`app/src/lib/i18n/th.ts`) พร้อมระบบ Agent System Prompt ที่ตอบกลับเป็นภาษาไทยอย่างเป็นธรรมชาติ
2. **Local AI ทำงานในเครื่อง (Ollama)**: เชื่อมต่อโมเดล `qwen3.5:4b` และ Embeddings `nomic-embed-text` ทำงานได้แม้ไม่มีอินเทอร์เน็ต และไม่มีค่าใช้จ่าย API รายเดือน
3. **คลังความรู้ส่วนบุคคล (Obsidian Vault)**: บันทึกและดึงข้อมูลความจำของ Agent ผ่าน `D:\openhuman-local\OpenHuman-Obsidian` โดยตรง
4. **เปิดใช้งานในคลิกเดียว (One-Click Launcher)**: ดับเบิลคลิก `Start-OpenHuman-Thai.bat` เพื่อเปิดทั้ง Ollama Server, Core Backend และ Desktop UI พร้อมกันทันที
5. **ปลดล็อกการตั้งค่าผ่านเบราว์เซอร์**: สามารถปรับแต่ง Model, Agent Autonomy, Permissions และ Memory Context ได้อย่างอิสระ

---

## 📚 เอกสารคู่มือที่สำคัญ (Documentation)

* 📖 **[คู่มือติดตั้งในเครื่องใหม่ (NewINSTALL.md)](./NewINSTALL.md)**: ขั้นตอนการ Setup และ Install สำหรับนำไปใช้กับอุปกรณ์เครื่องอื่น
* ⚙️ **[คู่มือการตั้งค่าและ Environment (.ENV)](./OPENHUMAN_SETUP_CONFIG_GUIDE_TH.md)**: รายละเอียดตัวแปรสภาพแวดล้อม พอร์ตการเชื่อมต่อ และการตั้งค่า AI ทั้งหมด
* 🤖 **[Prompt สั่งงาน AI Agent อัตโนมัติ (PROMPT_AGENTS.md)](./PROMPT_AGENTS.md)**: คำสั่ง Prompt สำหรับให้ Antigravity `agy` ดำเนินการ Setup เครื่องใหม่อัตโนมัติ
* 📜 **[คำสั่งสำหรับ Agent และนักพัฒนา (AGENTS.md)](./AGENTS.md)**: ข้อกำหนดและสถาปัตยกรรมระดับ Core

---

## 🚀 เริ่มต้นใช้งานด่วน (Quick Start)

1. **เปิดโปรแกรม**:
   * ดับเบิลคลิกไอคอน **`OpenHuman (Thai)`** บนหน้า Desktop หรือ
   * ดับเบิลคลิกไฟล์ **`Start-OpenHuman-Thai.bat`**
2. **เข้าสู่ระบบครั้งแรก (BootCheckGate)**:
   * **Runtime URL**: `http://127.0.0.1:7788/rpc`
   * **Auth Token**: `openhuman-local-token-12345`
   * กด **Test Connection** ➔ กด **Continue**
3. **ทดสอบพิมพ์แชท**:
   * พิมพ์ทักทายภาษาไทยในหน้าต่าง Chat เช่น: *"สวัสดีครับ แนะนำตัวเองเป็นภาษาไทยหน่อยครับ"*
