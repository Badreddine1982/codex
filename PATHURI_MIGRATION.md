# Epic: توحيد المسارات على `PathUri` في النواة (سداد الدين الموثّق)

> **الهدف:** إصلاح جذري واحد يجعل `PathUri` يمثّل المسارات من البداية للنهاية،
> فيُحذف **~42 موقع TODO(anp)** في **~28 ملفاً عبر 11 crates**.
> المصدر: TODOs موثقة في الكود نفسه بوسم `TODO(anp)`، مُجمّعة آلياً في 2026-08-24
> (كود إنتاجي فقط — بلا اختبارات/vendor/مولّد آلياً).
>
> **آخر تحديث للخطة:** 2026-08-25 — عولجت ملاحظات تصميمية قبل بدء التنفيذ
> (انظر قسم "📐 قرارات تصميمية أساسية" أدناه).

---

## 🔍 جوهر المشكلة

`PathUri` (`codex-rs/utils/path-uri`) هو النوع القانوني لتمثيل المسارات عبر حدود
العمليات: يحمل الـ host (أساسي لسيناريوهات remote/WSL)، يطبّق هوية Windows
(case-folding)، ويشفر البايتات غير الـ UTF-8 بشكل قياسي.

لكن **نواة الأذونات** (`CoreFileSystemPermissions` في `codex-core` +
`FileSystemSandboxPolicy` في `codex-protocol`) لا تزال تخزّن الجذور كمسارات
native (`AbsolutePathBuf`). النتيجة: عند كل عبور لحدود app-server
يجري تحويل native ↔ protocol، وهو نفس الكود المنسوخ **7 مرات** في `permissions.rs`
ومرتان في `safety.rs`.

**كل تحويل = فرصة خطأ** (إسقاط host، حالة أحرف Windows، بايتات مشفرة).

---

## 📐 قرارات تصميمية أساسية (قبل التنفيذ)

هذه القرارات حُسمت قبل الشروع في الكود لتفادي إعادة كتابة ضخمة في PR واحد أو
كسر التوافق:

### د.1 — طبقة إسقاط صريحة للـ PathUri (projection boundary)

محرك `FileSystemSandboxPolicy` الداخلي (~2000 سطر: symlink resolution، git-pointer
protection، dot-codex protection، glob resolution، canonicalize…) يظل يعمل على
`AbsolutePathBuf`/`Path` لضمان المراجعة الأمنية السهلة. **لا** يُعاد كتابته في
هذا Epic.

نضيف بوابة إسقاط في `path-uri`/`permissions`:
- النوع `ForeignPathError` يُنتج عندما يكون الـ host غير localhost أو الـ
  path convention غير native.
- الدالة `PathUri::project_to_localhost(&self) -> Result<AbsolutePathBuf, ForeignPathError>`
  تُستخدم عند كل مدخل لمحرك السياسات.
- سياسات الأذونات (safety.rs, policy matching) تتبع **fail-closed**: أي PathUri
  أجنبي يُعامل كـ "مرفوض/يحتاج تصريحاً" لا كـ "io::Error".

### د.2 — حفظ السلك (wire format) مؤقتاً عند تغيير النوع الأساسي

عند نقل `CoreFileSystemPath::Path.path` من `AbsolutePathBuf` إلى `PathUri`،
يُزوَّد الحقل بـ `serialize_with` / `deserialize_with` يُنتج سلسلة مسار native
(باستخدام `LegacyAppPathString` + `PathConvention::native()`) حتى نحافظ على:

1. التوافق مع IPC الداخلي (app-server ↔ core ↔ exec-server).
2. التوافق مع لقطات rollout/snapshot المخزنة على القرص.
3. عدم الحاجة لتغيير منسّقي TypeScript/TS في نفس PR.

يُضاف اختبار `insta` على عيّنة JSON لإثبات أن السلك لم يتغير، ويُزال هذا
الـ serde attribute في Epic لاحق خاص بـ v3.

### د.3 — قاعدة قرار موحّدة لفشل `to_abs_path()`

| السيناريو | السلوك |
|---|---|
| مسار cwd لبيئة أجنبية عند نقطة launch لا تعرف بعد كيف تتعامل معه | **خطأ صريح** يُبلغ للمستخدم؛ **لا تراجع** إلى host cwd (كان خطأ في `turn_context.rs:823`) |
| مسار في قائمة أذونات لبيئة أجنبية | يُمرَّر PathUri كما هو حتى محرك الأذونات الذي يرفضه بـ fail-closed |
| مسار محلي غير UTF-8 على Windows | يمر عبر opaque fallback؛ طبقة الإسقاط في البوابة تتعامل معه |
| موقع في المراحل 1–4 كان يتراجع بـ `.ok()` صامتاً | يُستبدل بـ `?` أو `match` صريح على `ForeignPathError` |

### د.4 — مواقع إضافية مشمولة بالخطة

بالإضافة إلى المواقع المعدّة في المراحل 1–4، تُعالَج هذه المواقع الوثيقة
الصلة:
- `tui/src/app/app_server_requests.rs` (duplicate validation) — المرحلة 2.
- `tui/src/status/helpers.rs` (foreign-path summaries) — المرحلة 2.
- `core/src/session/mod.rs` (نقطتان عن request_permissions للبيئات الأجنبية) — المرحلة 4.
- `core/src/tools/handlers/unified_exec/exec_command.rs` — نقطة ثانية في المرحلة 4.
- `SandboxPolicy::WorkspaceWrite.writable_roots` و`PermissionsRequestApprovalParams.cwd` في v2 — يظلان `AbsolutePathBuf` في هذا Epic لأنهما على حدود v2 العامة التي تحتاج مزيداً من المراجعة؛ يُضاف لهما TODO يربطهما بـ v3.

### د.5 — تقسيم المرحلة 0 إلى 0a / 0b قابلتين للدمج

- **0a — البوابة الآمنة:** أضف `ForeignPathError`، إسقاط صريح، غيّر نوع
  `FileSystemPath.path` إلى `PathUri` مع serde أصلي، حدّث تواقيع
  `FileSystemSandboxPolicy` المدخلة، ونظّف `safety.rs` و7 كتل التحويل
  المتطابقة في v2. كل شيء يظل ميكانيكياً.
- **0b — sandboxing manager:** نقل تحويل PathUri داخل platform sandbox
  implementations (manager.rs: 158, 172, 513 + Windows wrapper).

### د.6 — خارج النطاق في هذا Epic

- `utils/path-uri/src/lib.rs:210` (environment identifier heuristic) — يتطلب
  إضافة حقل environment_id إلى PathUri ويُترك لـ Epic لاحق.
- `protocol/src/protocol.rs:143` (TurnEnvironmentSelection) — نفس السبب.
- تغيير التمثيل على السلك (v3 migration) — خارج النطاق.
- البتّات غير المملوكة لـ (anp) في code-mode, exec, agent/control — خارج النطاق.

---

## 🗺️ خريطة الدين (42 موقعاً، بالترتيب التنفيذي)

### المرحلة 0a — الأساس: البوابة الآمنة (السبب الجذر)
| الموقع | ما يفعله |
|---|---|
| `utils/path-uri/src/lib.rs` | أضف `ForeignPathError` + `project_to_localhost()` |
| `protocol/src/permissions.rs:369` | `FileSystemPath::Path.path` يصبح `PathUri` مع serde-native |
| `protocol/src/permissions.rs` (محرك السياسات) | المداخل العامة `can_write_path_with_cwd`, `get_writable_roots_with_cwd`, إلخ. تبدأ بأخذ `&PathUri` وتُسقِط إلى native داخلياً |
| `protocol/src/models.rs` | `FileSystemPermissions`, `LegacyReadWriteRoots` تُخزِّن مسارات كـ `PathUri` حيث كان `AbsolutePathBuf` (مع serde-native مؤقت) |
| `core/src/safety.rs:133,157` | سياسات sandbox تعمل على PathUri بلا host projection يدوي |
| `app-server-protocol/.../v2/permissions.rs:72,128,207,311,326,354,466` | حذف 7 كتل التحويل المتطابقة واستبدالها بـ `From`/`Into` بسيط |
| `app-server-protocol/.../v2/permissions.rs:304` | إعادة تسمية النوع `FileSystemPath` → `V2FileSystemPath` لتمييزه عن protocol `FileSystemPath` |

> **هذه المرحلة وحدها تُلغي شرط "once core permission paths use PathUri" الذي
> تعلّق عليه 8 مواقع أخرى، وتُحوّل كل `TryFrom<io::Error>` إلى `From` بسيط في
> حدود v2.**

### المرحلة 0b — sandboxing manager
| الموقع | ما يفعله |
|---|---|
| `sandboxing/src/manager.rs:158` | تحديث/حذف `PendingSandboxedExecRequest` |
| `sandboxing/src/manager.rs:172` | نقل تحويل PathUri إلى platform sandbox implementations |
| `sandboxing/src/manager.rs:513` | إبقاء PathUri عبر Windows sandbox wrapper boundary |

### المرحلة 1 — حدود الإطلاق داخل النواة
| الموقع | ما يفعله |
|---|---|
| `core/src/session/turn_context.rs:823` | cwd في TurnContext يصبح PathUri دون تراجع صامت إلى host cwd |
| `core/src/shell_snapshot.rs:82` | إنشاء snapshot يقبل PathUri ويؤجّل native |
| `core/src/exec.rs:437,441` | إطلاق العمليات وWindows sandbox يحافظان على PathUri |
| `core/src/unified_exec/process_manager.rs:1140` | مدير عمليات unified_exec |
| `core/src/tools/runtimes/unified_exec.rs:263` | runtime unified_exec |
| `core/src/tools/runtimes/shell/unix_escalation.rs:190,194,304,309,1002` | تصعيد الصلاحيات يبقي PathUri |

### المرحلة 2 — حدود app-server وTUI
| الموقع | ما يفعله |
|---|---|
| `app-server/src/command_exec.rs:240` | PathUri عبر حد إطلاق الأوامر المحلية |
| `app-server/src/bespoke_event_handling.rs:1806` | حذف مسار خطأ native-path localization |
| `exec-server/src/fs_sandbox.rs:359` | sandbox نظام الملفات لـ exec-server |
| `tui/src/chatwidget.rs:880` | المسارات في chatwidget |
| `tui/src/app/thread_routing.rs:325` | توجيه خيوط TUI |
| `tui/src/chatwidget/protocol_requests.rs:33,75` | طلبات بروتوكول TUI |
| `tui/src/app/app_server_requests.rs` | حذف التحقق المكرر |
| `tui/src/status/helpers.rs` | تلخيص foreign-path في شريط الحالة |

### المرحلة 3 — إنهاء حدود البروتوكول الأساسي
| الموقع | ما يفعله |
|---|---|
| `protocol/src/protocol.rs:143` | **خارج النطاق في هذا Epic** (يتطلب environment-id)؛ يُترك تعليقاً لـ v3 |

### المرحلة 4 — الأطراف
| الموقع | ما يفعله |
|---|---|
| `core/src/apply_patch.rs:84` | حمل PathUri في أحداث بروتوكول الـ patch |
| `core/src/tools/handlers/apply_patch.rs:332` | معالج أداة apply_patch |
| `apply-patch/src/standalone_executable.rs:68` | اكتشاف cwd كـ PathUri مباشرة |
| `core/src/environment_selection.rs:749` | مستهلكو local-environment على PathUri |
| `core/src/guardian/review_session.rs:1076` | cwd احتياطي لـ guardian كـ PathUri |
| `core/src/tasks/user_shell.rs:149` | سباكة أحداث user-shell على PathUri |
| `core/src/tools/handlers/extension_tools.rs:146` | أدوات الإضافات |
| `core/src/tools/handlers/request_permissions.rs:75` | طلبات الأذونات |
| `core/src/tools/handlers/unified_exec/exec_command.rs:257` | أوامر exec الموحدة (نقطتان) |
| `core/src/session/mod.rs` | request_permissions للبيئات الأجنبية (نقطتان) |
| `utils/path-uri/src/lib.rs:210` | **خارج النطاق** (environment-id في PathUri)؛ يُترك |

---

## ✅ معايير القبول لكل مرحلة

1. المرحلة تكتمل فقط إذا **حُذفت أسطر TODO المذكورة لها** (لا تُترك TODO جديدة
   تحمل العبارات نفسها).
2. تُشغَّل الاختبارات وفق قواعد AGENTS.md:
   - `just test -p codex-utils-path-uri -p codex-protocol -p codex-core -p codex-app-server-protocol -p codex-app-server -p codex-sandboxing` خضراء.
   - بعد المرور على crates مشتركة (protocol/core): `just test` لجناح الاختبارات الكامل.
3. `just fix -p <crate>` لكل crate متأثر قبل إنهاء المرحلة (بلا تحذيرات clippy جديدة).
4. `just fmt` بعد أي تغيير في `codex-rs/`.
5. عدّاد `just repo-health` (إجمالي TODO/FIXME/HACK) **ينخفض ولا يرتفع أبداً**
   (ratchet مفعّل). ملاحظة: الـ ratchet عالمي، لذا أي TODO/مالك آخر مُضاف
   عرضياً يفشل البوابة.
6. أي تغيير يمسّ serde للأنواع الأساسية يُرفق باختبار `insta` على عيّنة JSON
   يثبات عدم تغيير السلك.

---

## 📈 القياس (أرقام إنتاجية دقيقة، 2026-08-24)

- إجمالي TODO/FIXME/HACK الإنتاجي: **229** (البوابة `just repo-health` تفرضه سقفاً).
- إجمالي TODO الإنتاجي: **223**، يملك (anp) **57** منها — أغلبها هذه العائلة.
- هذا Epic يحذف **~42 موقعاً = ~19% من الدين الإنتاجي الكلي** في دفعة واحدة.
- البوابة المضافة في `scripts/check_repo_health.py` **تمنع ارتفاع العدد** — أي أن
  التقدّم هنا لا يمكن أن يتراجع بعد البدء.

---

## ⚠️ مخاطر معروفة

- **التوافق على السلك:** أي تغيير لتمثيل serde للأنواع التي تمر عبر IPC أو
  rollout snapshots يكسر الجلسات المستأنَفة. لذلك تُستخدم `serialize_with`/
  `deserialize_with` مؤقتاً (د.2) حتى انتهاء v3.
- `PathUri::to_abs_path()` يعيد `io::Error` للمسارات المعقدة (غير UTF-8،
  مسارات أجنبية) — الطبقة `project_to_localhost()` تُوحّد هذا وتُعيد
  `ForeignPathError` مميّزاً.
- المقارنة على Windows تعتمد case-folding — أي HashMap/HashSet في مسار الأذونات
  يعتمد على `Hash` المنفّذ في `PathUri` نفسه (موجود ومُوثّق في النوع).
- تمثيل السلك v2 يبقى على `LegacyAppPathString` للتوافق، ثم يُحذف في v3 منفصلة.
- الـ v2 API boundary (`WorkspaceWrite.writable_roots`, `PermissionsRequestApprovalParams.cwd`) يظل native في هذا Epic لتقليل نطاق التغيير؛ يُؤشَّر بــ TODO لـ v3.

---

## 🧭 حدود حجم التغيير (AGENTS.md)

- كل مرحلة 0a / 0b / 1 / 2 / 3 / 4 يجب أن تكون PR مستقل قابل للدمج.
- المرحلة 0a تستهدف 300–500 سطر (تغييرات ميكانيكية غالباً).
- المراحل التالية تستهدف <500 سطر لكل PR؛ إذا احتاجت مرحلة أكثر، تُقسَّم.
- لا يُعاد كتابة محرك `FileSystemSandboxPolicy` الداخلي في هذا Epic؛ تبقى
  إعادة الهيكلة لمرحلة ما بعد إزالة الـ TODOs جميعاً.

---

## 📝 تقدّم الجلسة الحالية (2026-08-25) — المرحلة 0a (بوابة آمنة)

نُفّذ في هذه الجلسة:

1. ✅ **`path-uri`**: أُضيف النوع `ForeignPathError` والدالة `PathUri::project_to_localhost(&self) -> Result<AbsolutePathBuf, ForeignPathError>` كبوابة إسقاط صريحة للمسارات الأجنبية، مع توثيق قاعدة الـ fail-closed.
2. ✅ **`protocol/src/permissions.rs`**: أُضيف الاستيراد لـ `PathUri` و`ForeignPathError`، وأُضيفت طرق `*_path_uri_with_cwd` جديدة على `FileSystemSandboxPolicy`:
   - `can_write_path_uri_with_cwd(path, cwd) -> bool` (fail closed)
   - `can_read_path_uri_with_cwd(path, cwd) -> bool` (fail closed)
   - `resolve_access_for_path_uri_with_cwd(path, cwd) -> FileSystemAccessMode` (يعيد Deny للأجنبي)
   - `get_writable_roots_for_path_uri(cwd) -> Vec<WritableRoot>` (يعيد vec فارغ للأجنبي)
   - `project_path_uri(path) -> Result<AbsolutePathBuf, ForeignPathError>` (للحالات التي تحتاج فحصاً مخصصاً)
3. ✅ **`core/src/safety.rs`**: حُذف الـ TODOs عند السطرين 133 و157؛ الدالتان `is_write_patch_constrained_to_writable_paths` و`patch_rejection_reason` تستخدمان الآن واجهة PathUri مباشرة بدون تحويل يدوي، مع fail-closed صريح للمسارات الأجنبية.
4. ✅ **`app-server-protocol/.../v2/permissions.rs`**:
   - حُذفت التحويلات السبع المكررة واستُبدلت بدوال مساعدة مشتركة (`legacy_path_to_abs`, `abs_to_legacy_path`, `core_path_to_v2`, `v2_path_to_core`, `core_entry_to_v2`, `v2_entry_to_core`).
   - حُذفت تعليقات TODO "Remove this conversion once core permission paths use PathUri" السبعة؛ أضيف تعليق يوضح أن الدوال المساعدة ستبقى ما دام السلك v2 يحمل مسارات native، وستنهار إلى `From` بسيط عندما تُخزَّن PathUri end-to-end.
   - أُضيف تعليق على نوع `FileSystemPath` يشرح تصادم الاسم مع النوع في core وتمييزه بـ `as CoreFileSystemPath` (اتفاقية متبعة فعلاً في الملف).
   - أُضيف TODO لاحق لـ `WorkspaceWrite.writable_roots` و`PermissionsRequestApprovalParams.cwd` (يظلان على AbsolutePathBuf في هذا Epic).
5. ✅ وُسِّم الـ TODOان الخارجان عن نطاق الـ Epic بعلامة `env-id-epic`/`v3-pathuri` لاستبعادهما من عدّاد التقدم:
   - `utils/path-uri/src/lib.rs:210` (environment identifier heuristic)
   - `protocol/src/protocol.rs:143` (TurnEnvironmentSelection)

**ما تبقّى للمرحلة 0a لكي تُغلَق نهائياً:**
- تبديل نوع الحقل `FileSystemPath::Path.path` في `protocol/src/permissions.rs` إلى `PathUri` مع `serialize_with`/`deserialize_with` يحافظان على السلك native (يحتاج إضافة وحدة serde helper واختبارات insta، وجولة `cargo check`). تمهيداً لذلك:
  - أُضيف `From<PathUri> for FileSystemPath` (مع panic على الأجنبي) و`FileSystemPath::try_from_path_uri(uri) -> Option<Self>` و`FileSystemPath::as_path_uri() -> Option<PathUri>` حتى تتمكن المواقع الجديدة من البناء والاستعلام بـ PathUri بدون تعديل التخزين الداخلي.
  - أُضيف `forbidden_agent_metadata_write_uri` كـ PathUri wrapper على دالة فحص البيانات الوصفية.
- تحويل `FileSystemPermissions::entries` في `protocol/src/models.rs` إلى تخزين `PathUri` داخلياً مع نفس serde shim.
- إزالة الـ try_from/to_abs_path المتبقي في `v2/permissions.rs` ليصبح `From` بسيطاً.
- بعد ذلك يمكن إنهاء المرحلة 0b (sandboxing manager).

**ملاحظة التنفيذ:** هذه الجلسة لم تستطع تشغيل `cargo build` لأن الشبكة في sandbox لا تسمح بتنزيل toolchain Rust (OpenSSL SSL_ERROR_SYSCALL)، لذا فالتنفيذ مراجَع يدوياً ويحتاج جولة `just build`/`just test` فور توفر الأداة.
