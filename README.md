<p align="center"><img src="assets/icon-512.png" width="120" alt="Gaza Man"></p>

<h1 align="center">Gaza Man 🇵🇸</h1>

<p align="center">
A simple, fast desktop API client, written entirely in Rust.<br>
Collections, environments, tests, JavaScript scripts, GraphQL, WebSocket and Socket.IO, in one small native app.
</p>

<p align="center">
<a href="../../releases/latest"><b>⬇ Download for Windows, macOS and Linux</b></a>
</p>

<p align="center"><img src="docs/screenshots/01-simple.png" alt="Gaza Man" width="900"></p>

برنامج ديسكتوب لتجربة الـ APIs، مكتوب بالكامل بـ Rust (باستخدام [egui / eframe](https://github.com/emilk/egui)).
تعمل فيه كولكشنز، وتجرب الـ APIs، وتكتب تيستات وسكريبتات، وتصدّر كود الريكويست بـ 17 لغة. وفيه وضع **Simple** للمبتدئين والطلاب.

## التحميل

من صفحة [**Releases**](../../releases/latest):

| النظام | الملف | التشغيل |
|---|---|---|
| **Windows** 10 / 11 | `Gaza-Man-…-windows-x86_64.zip` | فك الضغط وافتح **Gaza Man.exe** |
| **macOS** 11+ (Apple Silicon و Intel) | `Gaza-Man-…-macos-universal.zip` | فك الضغط وانقل **Gaza Man.app** لـ Applications. أول مرة اعمل كليك يمين ← **Open**، لأن التطبيق مش موقّع من Apple |
| **Linux** (x86_64) | `Gaza-Man-…-linux-x86_64.tar.gz` | فك الضغط وشغّل `./gaza-man`. محتاج GTK 3 (موجود في أغلب التوزيعات) |

لو Windows طلّع رسالة SmartScreen، اضغط **More info** ← **Run anyway**، لأن البرنامج جديد ومش موقّع.

## صور من البرنامج

| | |
|---|---|
| ![Light theme](docs/screenshots/02-light.png) **الوضع الفاتح** | ![Scripts and tests](docs/screenshots/03-scripts.png) **سكريبتات JavaScript وتيستات** |
| ![GraphQL suggestions](docs/screenshots/06-graphql.png) **اقتراحات GraphQL من الـ schema** | ![Socket.IO](docs/screenshots/05-socketio.png) **اتصال Socket.IO مباشر** |
| ![Visualizer](docs/screenshots/04-visualize.png) **pm.visualizer: الرد كجدول** | ![Image preview](docs/screenshots/07-image.png) **معاينة الصور في الرد** |
| ![Customize](docs/screenshots/08-customize.png) **اختار انت إيه اللي يظهر** | ![New request types](docs/screenshots/09-new.png) **HTTP و GraphQL و WebSocket و Socket.IO** |
| ![Runner](docs/screenshots/10-runner.png) **تشغيل الكولكشن كلها (Runner)** | |

## التشغيل من الكود

محتاج [Rust](https://rustup.rs) (1.85 أو أحدث):

```bash
cargo run --release
```

أو افتح ملف مباشرة (كولكشن v2 / v2.1 أو OpenAPI أو Insomnia أو HAR):

```bash
cargo run --release -- my-api.collection.json
```

على Linux محتاج المكتبات دي الأول:

```bash
sudo apt install libgtk-3-dev libxkbcommon-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libwayland-dev pkg-config
```

### تطبيق Mac تفتحه بدبل كليك

```bash
./make-app.sh            # للماك ده
./make-app.sh universal  # لكل أجهزة الماك (Apple Silicon و Intel)
```

هيعمل `Gaza Man.app` جوه الفولدر، تقدر تنقله لـ Applications.

### نسخة جديدة

اعمل tag برقم النسخة، و GitHub Actions بيبني نسخ Windows و Linux و macOS ويحطها في Releases لوحده:

```bash
git tag v2.0.0 && git push origin v2.0.0
```

## سهل أو كامل: انت اللي بتختار

زرار **Simple** فوق على اليمين بيغيّر شكل البرنامج:
- **Simple** (الافتراضي): الأساسيات بس. الـ URL والـ Params والـ Authorization والـ Headers والـ Body والرد والـ **Environments** والـ History.
- زرار العين 👁 جنب الـ Environment بيوريك كل المتغيرات مرة واحدة: بتاعة الكولكشن (زي `{{baseUrl}}`) وبتاعة الـ Environment والـ Globals، وجنب كل نوع زرار يعدّلها. والزرار ده بيفضل ظاهر حتى لو الـ Environments مقفولة.
- **Full**: كل المميزات.
- **Custom**: من نفس الزرار تقدر تفتح أو تقفل كل ميزة لوحدها، زي Tests و Scripts و Environments و Runner و Console و Cookies و Mock server و GraphQL و WebSocket و أدوات الشرح.

لما تقفل ميزة هي بتستخبى بس، ما فيش حاجة بتتمسح. ولو فيه ريكويست بيستخدمها فعلاً (فيه سكريبت مثلاً)، التبويب بتاعها بيفضل ظاهر فيه.

## ريكويست جديد

زرار **+ New** فيه كل الأنواع على طول: **HTTP** و **GraphQL** و **WebSocket** و **Socket.IO**، وكل واحد مكتوب جنبه هو بيعمل ايه. لو اخترت نوع مقفول في الوضع اللي انت فيه، البرنامج بيفتحه لوحده. وفيه كمان **Demo collection** ترجّع بيه الديمو في أي وقت.

## الديمو

كولكشن **Gaza Man Demo** معمولة على APIs حقيقية ومتاحة للكل، ومترقمة زي الدرس:
1. **Reading data**: GET و query params و path variables، على dummyjson.com، والطقس في غزة من Open-Meteo.
2. **Sending data**: POST و PUT و PATCH و DELETE و form-data.
3. **Logging in**: login بيحفظ الـ token، والريكويست اللي بعده بيستخدمه كـ Bearer.
4. **Images and pages**: رد فيه صورة، وصفحة HTML.
5. **GraphQL**: بيانات فلسطين من Countries API.
6. **Live connections**: WebSocket و Socket.IO على سيرفرات echo عامة.
7. **Scripts**: Pre-request و Post-response بـ JavaScript.

كل ريكويست مكتوب تحته بيعلّم ايه. والـ Runner بيشغل الكولكشن كلها، وبيعدّي الـ WebSocket والـ Socket.IO لأنهم اتصالات بتفضل مفتوحة.

## المميزات

### الكولكشنز
- **كولكشنز وفولدرات متداخلة** في شجرة جانبية: إضافة، إعادة تسمية، نسخ، حذف، ترتيب (Move up/down) و**سحب وإفلات** بين الفولدرات والكولكشنز.
- **بحث** في كل الريكويستات بالاسم أو الـ URL أو الـ Method (⌘K).
- **إعدادات الكولكشن والفولدر**: وصف، Authorization يورّث لكل اللي تحته، ومتغيرات الكولكشن.
- **Tabs** زي المتصفح: كل ريكويست في تاب، ونقطة برتقالي لو فيه تعديلات مش محفوظة، و**Save / Save as** (⌘S).

### الريكويست
- **Params**: جدول Query Params متزامن مع الـ URL، و**Path Variables** (اكتب `/:id` في الـ URL).
- **Authorization**: Inherit / No Auth / Bearer / Basic / API Key (Header أو Query) / **Digest** / **AWS Signature v4** / **NTLM** / **OAuth 2.0** (وفيه زرار يجيب Access Token بـ Client Credentials أو Password).
- **Headers** مع عرض الـ Headers اللي بتتولد أوتوماتيك.
- **Body**: none / form-data (نص **وملفات**) / x-www-form-urlencoded / raw (JSON, Text, JavaScript, HTML, XML) مع تلوين وتحقق من الـ JSON و Beautify / binary / **GraphQL**.
- **Bulk Edit** لأي جدول key:value.
- **المتغيرات** `{{name}}` بتتلون أخضر لو معرفة وأحمر لو لأ، وفيه **اقتراحات** وانت بتكتب `{{`، وتشوف الـ URL النهائي تحت الخانة.
- **Dynamic variables**: `{{$guid}}` `{{$timestamp}}` `{{$isoTimestamp}}` `{{$randomInt}}` `{{$randomEmail}}` `{{$randomFullName}}` وغيرهم.
- **الصق أمر cURL** في خانة الـ URL ويتحول لريكويست كامل.
- **Examples**: احفظ أي رد كمثال جوه الريكويست. بيظهر في التوثيق اللي بتعمله Export، والـ Mock server بيرد بيه.
- **Undo / Redo** (⌘Z / ⌘⇧Z) لو مسحت أو غيرت حاجة بالغلط.

### GraphQL
- زرار **Fetch schema** بيسأل السيرفر عن الـ schema بتاعه (introspection) بنفس الـ URL والـ Headers والـ Auth.
- **اقتراحات وانت بتكتب** حسب مكانك في الـ query: جوه `countries { }` يقترح حقول الـ `Country` بس، ومعاها نوع كل حقل ووصفه. Enter أو Tab تختار، والأسهم تتحرك، و Ctrl+Space يطلّع كل الاقتراحات. الحقول اللي جواها حقول بتتكتب ومعاها `{ }` والمؤشر جواها.
- **Schema explorer**: شجرة بكل الـ Query و Mutation وحقولهم، تدوس على حقل يتضاف في الـ query.
- المسافات بتتظبط أوتوماتيك لما تدوس Enter.

### WebSocket و Socket.IO
- اختار **WebSocket** أو **Socket.IO** من قايمة الـ Method جنب الـ URL، ودوس **Connect**.
- `ws://` و `wss://` (مع SSL)، والـ Headers والـ Auth بتتبعت في الـ handshake.
- **Socket.IO** v2 و v3 و v4: بيعرف إصدار السيرفر لوحده (أو اكتب `EIO=3` في الـ URL عشان تفرض v2). بيدخل الـ namespace (اكتبه في آخر الـ URL زي `/chat`)، وبيرد على الـ ping لوحده، وتكتب اسم الـ **Event** والـ data (JSON بيتبعت object، وأي حاجة تانية نص).
- **سجل الرسايل**: اللي بعته (أخضر ↑) واللي جالك (أزرق ↓) بالوقت، واسم الـ event، والـ JSON منسق وملون، وبحث في الرسايل، ونسخ، وتشوف الـ handshake headers.

### التيستات والمتغيرات (من غير كود)
- **Tests**: Status code / Response time / JSON path / Header / Body مع equals، greater than، contains، exists… وبيبان PASS / FAIL جنب كل تيست.
- **Variables (Capture)**: تحفظ جزء من الرد في متغير (زي التوكن بعد الـ Login) في الـ Environment أو الكولكشن أو Globals، والريكويستات اللي بعدها تستخدمه.
- لما تعمل Export للكولكشن، التيستات بتتحول لـ **سكريبتات `pm.test()` حقيقية** بتشتغل في أي أداة بتدعم الصيغة دي.

### الرد
- **Pretty / Raw / Tree** للـ JSON، وبحث بيلوّن النتايج، وفلترة بـ `$.path`.
- في الـ **Tree**: كليك يمين على أي قيمة ← Copy، أو **"Test: equals"**، أو **"Save to variable"**.
- Headers، **Cookies**، **Test Results**، الحجم والوقت، و**شرح لكل Status Code** (مفيد في الشرح للطلاب).
- **Preview**: الصور (PNG و JPEG و GIF و WebP و BMP و ICO و **SVG**) بتظهر صورة، والـ HTML بيظهر كـ **reader view** (العناوين والفقرات والقوايم واللينكات من غير التنسيق)، ومعاه زرار **Open in browser** يفتح الصفحة الحقيقية.
- حفظ الرد في ملف.

### الكود
- زرار `</>` بيطلّع كود الريكويست بـ **17 لغة**: cURL، JavaScript Fetch، Axios، Python Requests، Go، PHP cURL، Java HttpClient، C# HttpClient، Swift، Kotlin OkHttp، Dart، Ruby، Rust reqwest، PowerShell، HTTPie، wget، وHTTP خام. نسخ أو حفظ في ملف.

### Environments و Globals
- أكتر من Environment (زي Local و Production) وتبدّل بينهم من فوق، وزرار 👁 تشوف فيه المتغيرات بسرعة.
- الأولوية: Globals ← Collection ← Environment.

### Collection Runner
- يشغل كولكشن أو فولدر بالترتيب، مع **Iterations** و**Delay** و**Stop on first failure**.
- **Data file (CSV أو JSON)**: كل سطر في الملف لفة (iteration)، وأعمدته متغيرات تستخدمها في الريكويستات.
- ملخص (Requests passed، Tests passed، متوسط الوقت، المدة) وفلتر Passed / Failed، وتفاصيل كل تيست، و**Export للنتايج** JSON.
- المتغيرات اللي بتتحفظ من ريكويست بتوصل للي بعده، و`{{iteration}}` فيه رقم اللفة.

### Import / Export
- **Import**: كولكشنز v2 / v2.1، والـ Environments والـ Globals، **OpenAPI 3 / Swagger 2** (JSON أو **YAML**، وبيعمل أمثلة للـ Body من الـ schema)، **Insomnia**، **HAR** (اللي بتعمله Export من تبويب Network في المتصفح، والردود المسجلة فيه بتبقى Examples)، و**أمر cURL**. من زرار Import أو بالسحب على الشباك.
- **Export للكولكشن**: Collection v2.1، **OpenAPI 3**، **صفحة توثيق HTML** شكلها حلو، Markdown، سكريبت cURL، وملف `.http`.
- Export للـ Environment بصيغة JSON المعروفة.

### حاجات تانية
- **History** لكل ريكويست اتبعت، متقسمة بالأيام.
- **Cookie jar**: الكوكيز بتتحفظ وتتبعت تاني أوتوماتيك زي المتصفح، ومدير للكوكيز تمسح منه.
- **Console** (⌘⌥C) بيوريك الريكويست زي ما اتبعت بالظبط والرد.
- **Mock server محلي**: من صفحة الكولكشن، يرد على `localhost` بالـ Examples المحفوظة. مفيد للطلاب لما الـ API لسه ما اتعملش.
- **Settings**: Timeout، Follow redirects، SSL verification، **Proxy**، **شهادات العميل (mTLS)** وشهادة CA خاصة، الكوكيز، User-Agent، حجم الـ History.
  الإعدادات دي بتشتغل على WebSocket و Socket.IO كمان: Proxy من نوع HTTP أو SOCKS4 أو SOCKS5 (ولو فيه يوزر وباسورد)، وشهادات العميل، والـ CA.
- **ثيم داكن وفاتح**.
- كل حاجة بتتحفظ أوتوماتيك، ولو كان عندك كولكشن من النسخة القديمة هتلاقيه.

## أدوات الشرح للطلاب

- **العدسة 🔍** (زرار فوق أو ⌘/Ctrl + L): بتكبّر أي جزء من البرنامج تحت الماوس.
  - ⌥ Option + سكرول: تكبير وتصغير العدسة (من 1.25× لحد 8×)، أو `[` و `]`
  - ⌥ Option + ⇧ Shift + سكرول: تكبير وتصغير حجم العدسة نفسها
  - Esc: تقفل العدسة
- **تكبير البرنامج كله**: أزرار − / 100% / + فوق، أو ⌘/Ctrl + و ⌘/Ctrl − و ⌘/Ctrl 0.
- كولكشن **Gaza Man Demo** بتظهر أول مرة، فيها أمثلة لكل حاجة (متغيرات، Path params، تيستات، Capture، form-data، GraphQL).

## اختصارات الكيبورد

| الاختصار | الوظيفة |
|---|---|
| ⌘ Enter | Send |
| ⌘ S | Save |
| ⌘ T | ريكويست جديد |
| ⌘ W | قفل التاب |
| ⌘ ⇧ ] / [ | التاب اللي بعده / اللي قبله |
| ⌘ O | Import |
| ⌘ K | بحث |
| ⌘ B | إظهار / إخفاء الشريط الجانبي |
| ⌘ ⇧ R | Collection Runner |
| ⌘ ⌥ C | Console |
| ⌘ , | Settings |
| ⌘ L | العدسة |
| ⌘ Z / ⌘ ⇧ Z | Undo / Redo |
| Ctrl Space | كل اقتراحات GraphQL |

## الملفات

| الملف | وظيفته |
|---|---|
| `src/main.rs` | تشغيل الشباك |
| `src/app.rs` | حالة البرنامج (الكولكشنز والتابات) والأوامر وإرسال الريكويستات والـ Runner |
| `src/ui_sidebar.rs` | الشريط العلوي والجانبي (Collections، Environments، History) والتابات وشريط الحالة |
| `src/ui_request.rs` | محرر الريكويست والرد وبانل الكود |
| `src/ui_dialogs.rs` | صفحات الكولكشن والـ Environment، والنوافذ (Import، Save، Runner، Settings، Cookies، Console) |
| `src/collection.rs` | شكل البيانات، وقراءة وكتابة صيغة الكولكشن v2 / v2.1 |
| `src/http.rs` | المتغيرات والـ Auth والـ Body والإرسال والكوكيز |
| `src/tests.rs` | التيستات والـ Capture وتحويلهم لسكريبتات `pm.test` |
| `src/codegen.rs` | توليد الكود بـ 17 لغة |
| `src/import.rs` | الـ Import من الكولكشنز و cURL و OpenAPI و Insomnia و HAR |
| `src/yaml.rs` | قارئ YAML صغير لملفات OpenAPI |
| `src/graphql.rs` | الـ schema والاقتراحات حسب مكان المؤشر |
| `src/ws.rs` | عميل WebSocket و Socket.IO |
| `src/preview.rs` | معاينة الصور والـ HTML |
| `src/mock.rs` | الـ Mock server المحلي |
| `src/auth.rs` و `src/crypto.rs` | Digest و AWS Signature و NTLM والتشفير اللي محتاجينه |
| `src/data.rs` | قراية ملفات الـ CSV و JSON للـ Runner |
| `src/script.rs` | تشغيل السكريبتات بمحرك QuickJS |
| `src/pm.js` | الـ `pm.*` API نفسه، مكتوب بـ JavaScript |
| `src/pm_libs.js` | moment و cheerio و xml2Json و Handlebars (للـ visualizer) |
| `src/export.rs` | الـ Export لـ Collection v2.1 و OpenAPI و HTML و Markdown و cURL و `.http` |
| `src/theme.rs` | الألوان والخطوط والـ widgets الخاصة بالتصميم |
| `src/rtl.rs` | ترتيب الكلام العربي صح في العرض |
| `src/lens.rs` | العدسة المكبّرة |
| `src/shot.rs` | وضع للتصوير (screenshots) لاختبار الواجهة من سكريبت |
| `src/features.rs` | وضع Simple و Full، وإظهار أو إخفاء كل ميزة |
| `assets/fonts/Phosphor.ttf` | أيقونات [Phosphor](https://phosphoricons.com) (رخصة MIT، في `Phosphor-LICENSE.txt`) |
| `assets/icon.svg` | الأيقونة الأصلية. منها اتعمل `GazaMan.icns` و`window-icon.png` |

## الاختبارات

```bash
cargo test
```

## ملحوظة عن العربي

- **النصوص اللي بتتعرض** (أسماء الكولكشنز والريكويستات والردود وصفحات الـ HTML) بتظهر بالترتيب الصح.
- **الخانات اللي بتكتب فيها** (الـ URL والـ Params والـ Headers والـ Body والوصف):
  - **وانت مش بتكتب فيها**: العربي بيظهر بالترتيب الصح.
  - **وانت بتكتب**: بيظهر بترتيب الكتابة، وتحت الخانة **"Reads as"** بيوريك السطر اللي فيه المؤشر بالترتيب الصح.
- السبب: مكتبة egui نفسها لسه ما بتدعمش الكتابة من اليمين للشمال جوه خانة الإدخال.

## السكريبتات (JavaScript)

تبويب **Scripts** في كل ريكويست وفولدر وكولكشن، فيه **Pre-request** (قبل ما الريكويست يتبعت) و**Post-response** (بعد ما الرد يوصل)، مع تلوين للكود وقايمة **Snippets** تدوس عليها تتكتب.

بتشتغل بالترتيب ده: سكريبت الكولكشن، بعده الفولدر، بعده الريكويست. والمحرك هو QuickJS، فالـ JavaScript الحديثة شغالة كلها (arrow functions و async/await و destructuring...).

**الأوامر المدعومة:**
- **المتغيرات**: `pm.environment` و `pm.collectionVariables` و `pm.globals` و `pm.variables` و `pm.iterationData`، وفيهم `get` و `set` و `unset` و `has` و `toObject` و `clear` و `replaceIn`.
- **التيستات**: `pm.test(name, fn)` و `pm.test.skip`، و `pm.expect` بأسلوب Chai: `to.equal` و `eql` و `deep.equal` و `include` و `have.property` و `lengthOf` و `a("array")` و `above` و `below` و `match` و `oneOf` و `ok` و `true` و `null` و `exist` و `empty` و `not`...
- **الرد**: `pm.response.code` و `status` و `responseTime` و `json()` و `text()` و `headers.get()`، و `pm.response.to.have.status(200)` و `to.be.ok` و `to.have.header()` و `to.be.json`.
- **الريكويست**: `pm.request.url` و `method` و `headers.add / upsert / remove` و `body.raw`. التعديلات اللي بتعملها في Pre-request بتتبعت فعلاً.
- **`pm.sendRequest`**، سواء بـ callback أو بـ `await`، زي إنك تجيب توكن قبل الريكويست.
- **`console.log`** بيظهر في Test Results وفي الـ Console.
- **`CryptoJS`** (MD5 و SHA1 و SHA256 و Hmac و Base64)، و `atob` / `btoa`، و `require('crypto-js')` و `require('uuid')`.
- **lodash** (`_` أو `require('lodash')`): أشهر الدوال، زي `get` و `set` و `pick` و `omit` و `map` و `filter` و `find` و `sortBy` و `orderBy` و `groupBy` و `countBy` و `uniq` و `isEqual` و `isEmpty` و `cloneDeep` و `merge` و `range` و `sum` و `camelCase`.
- **في الـ Runner**: `pm.execution.setNextRequest(name)`، و `pm.execution.skipRequest()`.
- **الصيغة القديمة**: `tests["..."] = true` و `responseBody` و `responseCode` وأوامر `setEnvironmentVariable` القديمة.

**الحماية:** أي سكريبت لازم يخلص في 10 ثواني (عشان لو فيه loop مالوش آخر البرنامج ما يهنجش)، وبحد أقصى 64 MB ذاكرة.

- **`moment`**: `format` و `add` و `subtract` و `diff` و `startOf` و `endOf` و `isBefore` و `isAfter` و `utc` و `unix` و `fromNow`.
- **`cheerio`**: تقرا HTML بـ CSS selectors (`#id` و `.class` و `[attr]` و `>` و `:first-child`...). وفيه `text()` و `attr()` و `html()` و `find()` و `each()` و `map()`.
- **`xml2Json(responseBody)`**: يحوّل XML لـ object (الـ attributes في `$`).
- **`pm.visualizer.set(template, data)`**: قالب Handlebars (`{{x}}` و `{{#each}}` و `{{#if}}`)، والنتيجة بتظهر في تبويب **Visualize** في الرد. الجداول بتظهر أعمدة مترتبة، وزرار **Open in browser** بيفتح الصفحة الحقيقية، وفيها `pm.getData` عشان سكريبتات القالب تشتغل.
- **`pm.vault`**: أسرار زي الـ API keys والباسوردات، محفوظة على الجهاز بس وعمرها ما بتطلع مع كولكشن. بتعدّلها من **Settings → Vault**، وبتستخدمها كـ `{{vault:name}}` أو `await pm.vault.get("name")`.
- **`pm.cookies.jar()`**: فيه `get` و `getAll` و `set` و `unset` و `clear` على الكوكيز الحقيقية بتاعة البرنامج.

لو `pm.environment.set` اتنادى من غير Environment شغال، القيمة بتتحفظ في الـ Globals.
