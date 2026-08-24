# Epic: توحيد المسارات على `PathUri` في النواة (سداد الدين الموثّق)

> **الهدف:** إصلاح جذري واحد يجعل `PathUri` يمثّل المسارات من البداية للنهاية،
> فيُحذف **42 موقع TODO(anp)** في **22 ملفاً عبر 10 crates**.
> المصدر: TODOs موثقة في الكود نفسه بوسم `TODO(anp)`، مُجمّعة آلياً في 2026-08-24
> (كود إنتاجي فقط — بلا اختبارات/vendor/مولّد آلياً).

---

## 🔍 جوهر المشكلة

`PathUri` (`codex-rs/utils/path-uri`) هو النوع القانوني لتمثيل المسارات عبر حدود
العمليات: يحمل الـ host (أساسي لسيناريوهات remote/WSL)، يطبّق هوية Windows
(case-folding)، ويشفر البايتات غير الـ UTF-8 بشكل قياسي.

لكن **نواة الأذونات** (`CoreFileSystemPermissions` في `codex-core`) لا تزال تخزّن
الجذور كمسارات native (`AbsolutePathBuf`). النتيجة: عند كل عبور لحدود app-server
يجري تحويل native → protocol، وهو نفس الكود المنسوخ **7 مرات** في `permissions.rs`.

**كل تحويل = فرصة خطأ** (إسقاط host، حالة أحرف Windows، بايتات مشفرة).

## 🗺️ خريطة الدين (42 موقعاً، بالترتيب التنفيذي)

### المرحلة 0 — الأساس (سبب الجذر)
| الموقع | ما يفعله |
|---|---|
| `core/src/safety.rs:133` | سياسات sandbox تعمل على PathUri بدل native |
| `core/src/safety.rs:157` | فحوص المسار تقبل PathUri بلا host projection |
| `protocol/src/permissions.rs:369` | أنواع أذونات البروتوكول على PathUri |
| `sandboxing/src/manager.rs:158,172,513` | مدير الـ sandbox يقبل PathUri |

> **هذه المرحلة وحدها تُلغي شرط "once core permission paths use PathUri" الذي
> تعلّق عليه 8 مواقع أخرى.**

### المرحلة 1 — حدود الإطلاق داخل النواة
| الموقع | ما يفعله |
|---|---|
| `core/src/session/turn_context.rs:823` | cwd في TurnContext يصبح PathUri |
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

### المرحلة 3 — حصاد الجزء الأكبر (7 تحويلات متطابقة)
| الموقع | ما يفعله |
|---|---|
| `app-server-protocol/.../v2/permissions.rs:72,128,207,311,326,354,466` | **حذف 7 كتل تحويل متطابقة** |
| `app-server-protocol/.../v2/permissions.rs:304` | إعادة تسمية النوع لتمييزه عن protocol FileSystemPath |
| `protocol/src/protocol.rs:143` | أنواع البروتوكول الأساسية |

### المرحلة 4 — الأطراف (لا تعتمد على المراحل 0–3 بالكامل)
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
| `core/src/tools/handlers/unified_exec/exec_command.rs:257` | أوامر exec الموحدة |
| `utils/path-uri/src/lib.rs:210` | تنظيف داخلي في النوع نفسه |

## ✅ معايير القبول لكل مرحلة

1. المرحلة تكتمل فقط إذا **حُذفت أسطر TODO المذكورة لها** (لا تُترك TODO جديدة).
2. `cargo test -p codex-core -p codex-app-server -p codex-app-server-protocol -p codex-protocol -p codex-sandboxing` خضراء.
3. `cargo clippy --workspace` بلا تحذيرات جديدة.
4. عدّاد `just repo-health` للـ TODOs **ينخفض ولا يرتفع أبداً** (ratchet مفعّل).

## 📈 القياس (أرقام إنتاجية دقيقة، 2026-08-24)

- إجمالي TODO/FIXME/HACK الإنتاجي: **229** (البوابة `just repo-health` تفرضه سقفاً).
- إجمالي TODO الإنتاجي: **223**، يملك (anp) **57** منها — أغلبها هذه العائلة.
- هذا Epic يحذف **42 موقعاً = ~19% من الدين الإنتاجي الكلي** في دفعة واحدة.
- البوابة المضافة في `scripts/check_repo_health.py` **تمنع ارتفاع العدد** — أي أن
  التقدّم هنا لا يمكن أن يتراجع بعد البدء.

## ⚠️ مخاطر معروفة

- `PathUri::to_abs_path()` يعيد `io::Error` للمسارات المعقدة (غير UTF-8، أجهزة
  Windows) — كل موقع ترحيل يجب أن يقرر: خطأ صريح أم `from_abs_path` الآمنة
  (التي تشفر المسارات المستحيلة بدل الفشل).
- المقارنة على Windows تعتمد case-folding — أي HashMap/HashSet في مسار الأذونات
  يعتمد على `Hash` المنفّذ في `PathUri` نفسه (موجود ومُوثّق في النوع).
- الترحيل يلمس بروتوكول v2 المتسلسل (ser/de) — أبقِ تمثيل السلك كما هو
  (`LegacyAppPathString` يبقى للتوافق، ثم يُحذف في v3 منفصلة).
