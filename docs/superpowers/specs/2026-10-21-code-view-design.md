# Code view: duyệt working folder và review diff trong chat

- Ngày: 2026-10-21
- Nhánh: `feat/code-view`
- Trạng thái: thiết kế đã duyệt, chờ plan
- Mockup: `.superpowers/brainstorm/*/content/panel-hifi.html` (ảnh chụp đã duyệt)

## 1. Mục tiêu

Khi agent sửa code trong working folder của một thread (qua coder tools hoặc provider Claude Code),
người dùng xem được **agent đã sửa những gì** ngay cạnh cuộc chat, không phải mở editor khác chỉ để
kiểm tra. Hiện OpenHuman chỉ hiện tool card, không có cây file hay diff.

Đích đến: dùng riêng trên fork, nhưng tuân thủ quy ước của repo (domain module, test tách file,
shadcn primitives, i18n đủ 16 locale) để có thể gửi PR lên upstream về sau.

## 2. Phạm vi

### Có trong bản đầu

- Panel **Code** bên phải trong màn chat, gắn với working folder của thread đang mở.
- Tab **Changes**: các file khác với `HEAD` (git), kèm trạng thái và số dòng `+`/`−`; chọn file để xem diff.
- Tab **Files**: cây thư mục mở từng cấp; chọn file để xem nội dung (chỉ đọc).
- Tô màu cú pháp cho cả diff và nội dung file.
- Nút **Mở trong VS Code** tại file đang xem.
- Link **xem diff** trên tool card sửa file trong chat.
- Tự làm mới sau mỗi tool call sửa file, cộng nút làm mới tay.

### Không có trong bản đầu (thêm khi có nhu cầu thật)

- Sửa file, revert, stage, commit trong app.
- File watcher của hệ điều hành.
- Lọc theo `.gitignore` trong tab Files; tìm kiếm trong file.
- Diff so với mốc khác `HEAD` (ví dụ chỉ thay đổi của riêng thread).
- Trang `/code` toàn màn hình.

## 3. Trải nghiệm người dùng

Bố cục A (hai cột trong panel), như mockup đã duyệt:

```text
┌ Chat ─────────────────────────┐┌ Code ─────────────────────────────────────────┐
│ Sửa lỗi đăng nhập  [Code 3][⋯]││ 📁 ~/projects/shop-api  ⎇ fix-login     [↻][✕] │
│                               ││ (Changes 3) (Files)                           │
│ ... tin nhắn ...              ││ M auth.rs      +6 −2 │ src/auth.rs +6 −2 [↗ VS Code] │
│ ✎ edit src/auth.rs  xem diff ✓││ A token.rs       +31 │ @@ -38,9 +38,13 @@ ...       │
│ ✎ edit tests/...    xem diff ✓││ M auth_test.rs +12 −1│ 39 39   let user = ...      │
│                               ││ ? notes.md           │ 40    − if user.pw == pw {  │
│ [Nhắn cho OpenHuman…]         ││                      │    40 + if verify(...) {    │
└───────────────────────────────┘└───────────────────────────────────────────────┘
```

- Nút **Code** trên header của thread bật/tắt panel; số trên nút là số file thay đổi (ẩn khi bằng 0
  hoặc không phải git repo).
- Mỗi lúc chỉ có **một panel bên phải**: mở Code thì đóng panel Artifact và ngược lại; panel mở sau thắng.
- Panel hẹp dưới 520px: cột danh sách thu thành dải hẹp (chỉ chữ cái trạng thái và tên file).
- Đổi sang thread khác thì panel theo thread mới; file đang chọn bị bỏ chọn nếu không còn trong danh sách.
- Link **xem diff** trên tool card: mở panel, tab Changes, chọn đúng file đó.
- Footer: ngôn ngữ, mốc so sánh (`HEAD` + mã commit rút gọn), thời điểm cập nhật.

## 4. Backend: domain `code_view`

### 4.1 Vị trí và cấu trúc

`crates/openhuman-core/src/code_view/`, theo mẫu domain của repo:

| File | Việc |
| --- | --- |
| `mod.rs` | khai báo module, re-export, gom controller |
| `types.rs` | kiểu serde cho request/response |
| `ops.rs` | xác định root, chặn đường dẫn, `list_dir`, `read_file` |
| `git.rs` | gọi lệnh `git` (không dùng `git2`, đúng chủ trương của core) và parse output |
| `schemas.rs` | `ControllerSchema` + handler mỏng |
| `*_tests.rs` | test cạnh từng module, không có test inline |

Controller đăng ký trong `crates/openhuman-core/src/core/all.rs` cạnh nhóm workspace.

### 4.2 Root

Root của mọi RPC là working folder của thread, lấy bằng `thread_working_dir()` có sẵn trong
`threads/ops/working_dir.rs`. Thread chưa gán folder thì dùng `action_dir` chung của config. Root đi
qua cùng kiểm tra với `validate_working_dir()`: đường dẫn tuyệt đối, tồn tại, là thư mục,
canonicalize, không nằm trong vùng `SecurityPolicy::is_always_forbidden`.

### 4.3 RPC (namespace `code_view`, gọi là `openhuman.code_view_<function>`)

Mọi request đều có `thread_id: string`. `rel_path` là đường dẫn tương đối so với root, phân cách bằng `/`.

| Function | Request thêm | Response |
| --- | --- | --- |
| `root` | không có | `{ path, is_git, branch: string\|null, head: string\|null }`; `head` là mã commit rút gọn 7 ký tự, `null` khi repo chưa có commit |
| `list_dir` | `rel_path` (`""` là root) | `{ entries: [{ name, kind: "dir"\|"file"\|"symlink", size: number\|null }] }`, thư mục trước, rồi theo tên; luôn ẩn `.git` |
| `read_file` | `rel_path` | `{ content: string\|null, size, truncated, binary }`; `content` là `null` khi `binary` |
| `git_status` | không có | `{ files: [{ path, status: "M"\|"A"\|"D"\|"R"\|"U"\|"?", added: number\|null, deleted: number\|null, old_path: string\|null }] }` |
| `git_diff` | `rel_path` | `{ diff: string, truncated }` (unified diff, 3 dòng ngữ cảnh) |

Gợi ý ngôn ngữ để tô màu do **frontend** suy từ đuôi file; backend không trả về.

### 4.4 Bảo mật (ranh giới tin cậy)

- `rel_path`: phải là đường dẫn tương đối, không chứa NUL. Nối với root rồi canonicalize; kết quả
  phải nằm trong root (chặn `..` và symlink trỏ ra ngoài) và không thuộc vùng `is_always_forbidden`.
  Vi phạm thì trả lỗi `path not allowed`, không tiết lộ đường dẫn đã resolve.
- Chỉ đọc: không RPC nào ghi file hay làm thay đổi trạng thái git.
- Lệnh `git` luôn dùng tham số cố định và chạy với `-C <root>`; đường dẫn luôn đặt sau `--`.
  Luôn có `--no-ext-diff --no-textconv` (không chạy chương trình ngoài do repo cấu hình) và biến môi
  trường `GIT_OPTIONAL_LOCKS=0` (không giành khóa index khi agent đang chạy git). Timeout 10 giây.
- Giới hạn: `read_file` tối đa 1 MiB, `git_diff` tối đa 2 MiB; quá thì cắt và đặt `truncated: true`.
  File có byte NUL trong 8 KiB đầu là `binary`.

### 4.5 Git

- `root`: `git rev-parse --is-inside-work-tree`, `--abbrev-ref HEAD`, `--short=7 HEAD`. Không có `git`
  hoặc không phải repo thì `is_git: false`, không coi là lỗi.
- `git_status`: `git status --porcelain=v2 -z --untracked-files=all -- .` cộng
  `git diff HEAD --numstat -z -M -- .`. Trạng thái so với `HEAD` gộp cả staged lẫn unstaged; xung
  đột merge là `U`; file đổi tên có `old_path`. File chưa theo dõi (`?`) có `added` bằng số dòng nếu
  là file text dưới 1 MiB, ngược lại là `null`.
- Đường dẫn trong output git được chuẩn hóa về **tương đối so với root**, kể cả khi working folder
  chỉ là thư mục con của repo; các file nằm ngoài root không xuất hiện.
- `git_diff`: file tracked dùng `git diff HEAD -U3 -M -- <path>`; file chưa theo dõi dùng
  `git diff --no-index -U3 -- /dev/null <path>`. Repo chưa có commit thì so với empty tree
  (`4b825dc642cb6eb9a060e54bf8d69288fbee4904`).

## 5. Frontend

### 5.1 File mới (`app/src/features/codeView/`, mỗi file dưới 300 dòng)

| File | Việc |
| --- | --- |
| `codeViewApi.ts` | bọc 5 RPC qua `callCoreRpc`, kèm kiểu TypeScript khớp mục 4.3 |
| `parseUnifiedDiff.ts` | text diff → `hunks[{ header, lines[{ kind: ctx\|add\|del\|meta, oldNo, newNo, text }] }]` |
| `highlight.tsx` | `lowlight` → cây `<span>` React; bảng đuôi file → ngôn ngữ; không đoán ngôn ngữ tự động |
| `CodePanel.tsx` | khung panel: header, `ChipTabs`, chia hai cột, tải dữ liệu, tự làm mới |
| `ChangesList.tsx` | danh sách thay đổi (cột trái tab Changes) |
| `FileTree.tsx` | cây thư mục, mỗi cấp gọi `list_dir` khi mở (cột trái tab Files) |
| `DiffView.tsx` | diff có hai cột số dòng, tô màu cú pháp từng dòng |
| `FileView.tsx` | nội dung file chỉ đọc, có số dòng |
| `app/src/store/codeViewSlice.ts` | `{ open, threadId, tab, selectedPath, revision, changeCount }` |

`lowlight@3.3.0` (đã có sẵn trong lockfile, do `rehype-highlight` kéo theo) được khai báo thành
dependency trực tiếp của `app/`; không thêm thư viện nào khác.

### 5.2 Sửa file có sẵn (tối thiểu)

- `features/conversations/Conversations.tsx`: nút **Code** trên header thread; gắn `<CodePanel/>`
  cạnh chỗ gắn `ArtifactPreviewPanel`.
- `components/assistant-ui/elements/tool-fallback.tsx`: với tool sửa file (`edit`, `file_write`,
  `apply_patch`, và `Edit`/`Write`/`MultiEdit` khi chạy qua Claude Code) thì thêm link **xem diff**,
  lấy đường dẫn từ tham số `path` hoặc `file_path`; khi card chuyển sang hoàn tất thì tăng
  `codeViewSlice.revision`. Link lưu đường dẫn nguyên dạng vào `selectedPath`; `CodePanel` đổi
  đường dẫn tuyệt đối thành tương đối sau khi tải xong `root` (đường dẫn nằm ngoài root thì bỏ chọn).
  `apply_patch` không có đường dẫn rõ ràng thì link chỉ mở panel.
- `store/index.ts`: đăng ký slice. Loại trừ hai panel bằng `extraReducers` ở cả hai phía:
  `codeViewSlice` tự đóng khi `chatRuntime/openArtifact` chạy; `chatRuntimeSlice` thêm đúng một
  `extraReducers` đặt `activeArtifactId = null` khi `codeView/openPanel` chạy.
- Số trên nút **Code** (`changeCount`): nút tự gọi `git_status` khi thread được mở và khi `revision`
  đổi (debounce 400ms), kể cả lúc panel đang đóng; panel đang mở thì dùng chung kết quả của panel.
- `lib/i18n/*.ts`: khoảng 25 key `codeView.*`, dịch thật cho đủ 16 locale.
- `crates/openhuman-app/capabilities/default.json`: thêm `{ "url": "vscode://file/*" }` vào allowlist
  `opener:allow-open-url`. Nút VS Code gọi `openUrl("vscode://file/<root>/<rel_path>:<dòng>")`.

### 5.3 Luồng dữ liệu

```text
Nút Code / link "xem diff" ──dispatch──► codeViewSlice { open, threadId, selectedPath }
CodePanel (useSelector)
  ├─ mở panel / đổi thread ─► root + git_status ─► ChangesList
  ├─ chọn file (Changes) ───► git_diff ─► parseUnifiedDiff ─► DiffView
  ├─ tab Files ─► list_dir theo từng cấp ─► FileTree ─► chọn file ─► read_file ─► FileView
  └─ revision đổi ─► debounce 400ms ─► tải lại git_status + file đang xem
Tool card sửa file hoàn tất ──dispatch──► revision++
```

Response về sau một request mới hơn thì bị bỏ qua (so mã request), để làm mới dồn dập không vẽ
nhầm dữ liệu cũ.

### 5.4 Quy tắc UI của repo

Chỉ dùng primitive trong `components/ui/` (`Button`, `Badge`, `Alert`, `ChipTabs`…) và token trong
`styles/tokens.css`; màu thêm/xóa dùng `sage`/`coral`. Mọi chữ hiển thị đi qua `useT()`. Không dùng
dynamic import.

## 6. Xử lý lỗi

| Tình huống | Backend | Panel |
| --- | --- | --- |
| Folder của thread mất hoặc không truy cập được | `root` trả lỗi | `Alert` kèm đường dẫn + nút thử lại |
| Không phải git repo, hoặc máy không có `git` | `is_git: false` | chỉ tab Files, kèm một dòng giải thích; nút Code không hiện số |
| Đường dẫn ngoài root hoặc vào vùng cấm | lỗi `path not allowed` | `Alert` |
| File > 1 MiB / diff > 2 MiB | `truncated: true` | banner "chỉ hiện phần đầu" + nút VS Code |
| File nhị phân | `binary: true` | "File nhị phân, không hiển thị" |
| File bị xóa (D) / đổi tên (R) | diff toàn dòng xóa / có `old_path` | hiện `cũ → mới` |
| Git quá 10 giây | lỗi timeout | `Alert` + nút thử lại |

## 7. Kiểm thử

### Rust (`code_view/*_tests.rs`)

- Chặn đường dẫn: `..`, đường dẫn tuyệt đối, NUL, symlink ra ngoài root, root trong vùng cấm.
- `list_dir`: thứ tự sắp xếp, ẩn `.git`, symlink báo `kind: "symlink"`.
- `read_file`: cắt ở 1 MiB, nhận diện nhị phân.
- Parse `porcelain=v2 -z` và `--numstat -z` từ chuỗi mẫu (sửa, thêm, xóa, đổi tên, xung đột, chưa theo dõi).
- Một test dùng `git` thật trong repo tạm (bỏ qua nếu máy không có `git`): bao gồm working folder là
  thư mục con của repo và repo chưa có commit.

### Frontend (Vitest, đặt cạnh file nguồn)

- `parseUnifiedDiff`: nhiều hunk, số dòng, `\ No newline at end of file`, thông báo diff nhị phân.
- `highlight`: render ra span có class, ngôn ngữ không biết thì trả text thô.
- `codeViewSlice`: chỉ một panel tại một thời điểm, `revision`, đổi thread.
- `CodePanel` với API giả: hiện danh sách, chọn file hiện diff, không phải git thì chỉ có Files, lỗi hiện `Alert`.
- `tool-fallback`: link xem diff xuất hiện đúng với tool sửa file và chỉ với chúng.

**Kiểm tra theo chuẩn repo**: `pnpm typecheck`, `pnpm lint`, `pnpm format:check`, `pnpm i18n:check`,
`cargo check` và `cargo clippy` cho cả hai manifest, `pnpm rust:layout`.

**Kiểm tra thật**: build app, tạo thread có working folder là một git repo, cho agent sửa file, rồi
chụp ảnh panel để so với mockup.

## 8. Rủi ro và giả định

- Card của provider Claude Code: giả định tool call của Claude Code hiện thành card với tên
  `Edit`/`Write`/`MultiEdit` và tham số `file_path`. Nếu không đúng như vậy thì link xem diff không
  xuất hiện cho các card đó, nhưng panel và nút làm mới vẫn hoạt động. Kiểm tra trong bước kiểm tra thật.
- `Conversations.tsx` đã hơn 2300 dòng: chỉ thêm nút và chỗ gắn panel, mọi logic nằm trong `features/codeView/`.
- Tô màu từng dòng diff riêng lẻ có thể tô sai chuỗi hoặc comment nhiều dòng; chấp nhận cho bản đầu.

## 9. Điều chỉnh sau khảo sát code (trước khi lập plan)

Khảo sát chi tiết lúc lập plan cho thấy vài giả định trong mục 3 và 5 không khớp code thật. Các điều chỉnh:

1. **Không có header thread.** Tiêu đề thread chỉ hiện ở sidebar. Nút **Code** đặt ở thanh công cụ của ô soạn
   tin (`assistantComposerFooterExtras`, cạnh nút chọn reasoning effort), cộng một lệnh command palette
   "Open code panel" (`useRegisterAction`).
2. **Card tool thật là `OpenHumanToolCall` → `AssistantUiToolCallCard`, và card cố ý không đọc Redux** (để render
   được ở nơi không có store). Link **xem diff** đi qua prop `footer` có sẵn của card và chỉ phát một
   `CustomEvent` (`openhuman:code-view-open`) trên `window`; `Conversations.tsx` lắng nghe rồi `dispatch`.
3. **Tự làm mới không cần card dispatch:** `CodeViewSync` đếm số tool sửa file có `status === 'success'` trong
   `state.chatRuntime.toolTimelineByThread[threadId]`; số đếm đổi thì tải lại (debounce 400ms).
4. **Trạng thái git dùng chung:** slice giữ `root`, `files`, `loading`, `error`, `updatedAt` (thunk
   `loadCodeStatus`, bỏ qua response cũ theo `requestId`). Nút Code và panel cùng đọc từ đó; bỏ trường
   `changeCount` riêng.
5. **Tô màu cú pháp:** khai báo trực tiếp `lowlight@3.3.0` và `highlight.js@11.11.1` (cả hai đã có trong lockfile),
   đăng ký thêm `verilog`, `vhdl`, `tcl`, `dockerfile` ngoài bộ `common`. `FileView` cũng tô từng dòng, tối đa
   5000 dòng đầu.
6. **`core/all.rs` bị ghim 1216 dòng** trong `scripts/ci/check-openhuman-rust-layout.mjs`; thêm khối đăng ký 6 dòng
   nên nâng mốc ghim lên 1222 trong cùng commit.
7. **Hành động mở panel dùng chung** đặt ở `store/codeViewActions.ts` (`createAction`) để `chatRuntimeSlice` và
   `codeViewSlice` tham chiếu lẫn nhau mà không tạo vòng import.
