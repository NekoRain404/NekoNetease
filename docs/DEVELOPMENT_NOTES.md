# NekoNetease 开发注意事项

本文面向接手项目的开发者，记录日常开发中最容易造成回归、数据损坏或协议泄露的约束。架构决策请先读 [`docs/adr/`](adr/)，网易云功能路线请读 [`NETEASE_FEATURE_AUDIT.md`](NETEASE_FEATURE_AUDIT.md)，交付状态和工作区说明请读 [`PROJECT_HANDOFF.md`](PROJECT_HANDOFF.md)。本文不替代 `CLAUDE.md`、`CONTRIBUTING.md` 或代码中的模块文档。

## 开始前

- 项目跟随宿主 stable 工具链（最低 Rust 1.99）；`rust-toolchain.toml` 只声明 `stable` 与所需组件。
- 确认 `.cargo/config.toml` 存在且已纳入版本库。它提供 `NEKONETEASE_REPO_ROOT`，缺失会让测试 fixture 和编译期路径解析失败。
- 开发运行和测试使用独立数据目录。优先设置 `NEKONETEASE_DATA_DIR="$(mktemp -d)"`，不要拿真实用户数据库验证迁移或破坏性操作。
- 先看 `git status --short`。当前仓库可能同时存在其他团队的 UI、翻译或网易云改动，不要用 reset、checkout 或批量格式化覆盖它们。
- 大功能先读相关 ADR，再决定是否需要新增 ADR；不要因为“只是重构”就跳过架构决策记录。

## Cargo 与 crate 边界

- 根目录是虚拟 workspace，成员来自 `crates/*`，`winit/` 是独立 workspace。依赖版本集中在根 `Cargo.toml`，新增依赖应使用 workspace dependency。
- `nekonetease-views` 不得直接依赖 `nekonetease-store` 或 `nekonetease-net`。UI 访问数据库和网络必须经过 `nekonetease-app` 的 library/service API。
- crate 不互相 re-export 类型；调用处使用真实 crate 路径，例如 `nekonetease_ui::AppWindow`。
- 所有成员都继承 workspace lints。生产代码和测试都不能使用 `unwrap()`；优先使用 `?`、明确的 `expect("...")` 不变量，或 `let ... else`。
- 不要为了消除 warning 添加宽泛的 `#[allow(...)]`。优先删除无用代码；确实需要临时抑制时使用带原因的 `#[expect(..., reason = "...")]`。
- `.cargo/config.toml`、`Cargo.lock`、`rust-toolchain.toml` 和根 `Cargo.toml` 属于构建基线，改动后必须在干净环境重新验证。

## UI 与线程

- Slint event loop 只在主线程运行。后台任务不能直接调用 `ui.set_*`。
- 后台结果回 UI 使用 `slint::spawn_local`、`slint::invoke_from_event_loop` 或 `weak_handle.upgrade_in_event_loop`；需要等待 tokio future 时优先使用 `spawn_local(async_compat::Compat::new(...))`。
- UI 回调只提交意图，网络、数据库、扫描和文件操作放到 Tokio task。不要在 Slint callback 中做阻塞 I/O。
- 修改一个用户可见字符串时使用 `@tr(...)`，然后运行 `scripts/update-translations.sh`；不要手工编辑 `.pot`，也不要只更新一个语言目录。
- Rust view model、`.slint` property、callback 名称和测试必须一起修改。删除页面时同时清理加载状态、请求代次和 detail teardown，避免旧请求回写新页面。
- `VecModel` 等 Slint model 从后台更新时，先在 UI 线程升级弱句柄，再通过 `as_any().downcast_ref` 修改模型。

## 播放器与音频

- `PlayerState` 的状态锁必须在副作用前释放。改变状态并执行动作使用 `emit_and_execute()`；不要手写 `with_state_emit()` 后再在另一个路径调用 `execute_actions()`。
- 锁顺序保持 `exec_lock -> PlayerState -> decks -> output`，不能反向获取。
- 音频 callback 线程不能网络、分配大型对象、访问数据库、等待锁或写日志。设备恢复和健康检查放在后台任务。
- 云歌曲是短生命周期外部 source，暂停/恢复和本地文件的行为不同；不要把云 URL、cookie 或临时播放票据写入本地音轨表。
- 修改解码、采样率、声道、ReplayGain 或 crossfade 时，同时检查 `nekonetease-audio`、`nekonetease-playback`、`nekonetease-engine` 和对应测试，不要只在 UI 中修补表现。

## 数据库、文件和 migration

- 已发布 migration 不可修改；schema 变化新增时间戳 migration，并为升级、空库和旧库增加测试。
- SQLite 使用 WAL 和 SQLx migrations。不要直接复制 `nekonetease.db` 作为备份；备份逻辑必须遵循现有 store 实现。
- 路径使用 `Path::join`，不要拼接 `/` 或 `\\`。Windows 路径、UNC 路径和临时目录都必须能工作。
- 数据库写事务只在 `nekonetease-store` 内打开和组合；应用层调用 store API，不要把 SQLx transaction 穿过 crate 边界。
- artwork、歌词、队列和设置都有自己的缓存/持久化约定。新增缓存时明确上限、清理策略、原子写入和崩溃中断后的恢复行为。
- 任何日志都不能包含密码、cookie、token、签名 URL 或 session key。错误日志使用项目现有的 `describe` 脱敏链路，不要直接打印底层错误文本。

## 网易云协议

- 网易云接口是非官方协议，路径、加密、字段和权限可能变化。新增接口必须同时更新 endpoint registry、请求/解码测试和 `docs/netease-api.md`。
- 读请求和写请求的重试策略不同；写操作不能因为超时就盲目重试，否则可能重复收藏、评论或歌单编辑。
- 所有异步云请求都要考虑账号 generation、页面请求代次和取消；旧账号或旧页面的响应不能写入当前 UI。
- 网络层返回领域错误/状态，UI 不要解析网络错误字符串。至少区分无版权、试听、VIP、购买限制、地区限制、登录失效、接口失败和离线。
- 真实账号探针只使用专用测试账号。probe 输出计数和状态即可，禁止输出 cookie、密码或签名播放地址。
- 不要把“接口返回 HTTP 200”当成能力可用；必须检查业务 code、字段缺失、权限字段和实际播放行为。
- 网易云写操作、评论、MV、DJ、云盘和离线缓存必须先确认服务条款与内容授权范围。

## 测试与验证

最小验证顺序：

```bash
cargo fmt --all --check
cargo clippy --all-targets --locked --workspace -- -D warnings
cargo test --locked --workspace
cargo build --locked --release -p nekonetease
```

针对网易云改动再运行：

```bash
cargo test --locked -p nekonetease-net --lib
cargo test --locked -p nekonetease-app --lib cloud::tests
cargo run --locked -p nekonetease-net --example netease_probe
```

测试注意事项：

- 不要把网络可用性、真实账号登录和本地纯函数测试混在同一个单测中。
- 协议解码测试使用固定 JSON fixture；请求路径、协议类型、分页 offset/limit 和关键字段都要断言。
- UI 测试至少覆盖加载、空态、错误、关闭页面、切换账号和旧请求回写保护。
- 跨平台路径和平台条件必须在 Linux、Windows 代码路径中分别检查；不要因为本机是 Linux 就省略 Windows 兼容性。
- 定向测试被 Cargo 构建锁阻塞时，不要把“命令已启动”写成通过；记录实际退出码。
- `cargo check` 通过只代表能编译，不代表 Clippy、测试、发布构建或真实协议探针通过。

## Git 与交付

- 一个提交只做一个逻辑变更；使用 Conventional Commits 风格。
- 提交前运行 `git diff --check`，查看 `git diff --stat`，确认没有误带翻译生成物、截图、构建产物或其他团队改动。
- UI 功能提交应包含 `.slint`、Rust bridge、相关测试和翻译更新；网络功能提交应包含协议文档和解码测试。
- 不要提交真实账号信息、运行时数据库、日志、签名 URL、个人数据或本地绝对路径。
- 交接前明确区分：已提交基线、当前未提交工作树、已验证命令、未验证项目和真实账号手工检查项。
- 任何 README 功能声明都必须与实际 UI 入口和协议验证一致；实验接口标记为实验能力，不能写成完整支持。

## 推荐排查顺序

遇到问题时按边界定位：

1. 先看日志和 `AppError` 上下文，确认是 UI、应用服务、网络协议、数据库还是播放设备。
2. 如果是 UI 异常，检查请求代次、页面 teardown、UI 线程更新和 model 生命周期。
3. 如果是数据异常，检查 `Paths`、migration、事务边界、缓存原子写入和测试数据目录。
4. 如果是播放异常，检查外部云播放回调、source 生命周期、播放器状态锁和音频设备健康任务。
5. 如果是网易云异常，保存脱敏响应结构和业务 code，先补解码/协议测试，再改 UI 文案。

## 当前工作树提示

当前工作树仍未形成稳定提交，网易云改动、翻译文件和三处模块拆分同时在途。接手时先按 `git diff` 确认归属，不要把并行开发的结果当成基线。

- 已在本机验证：`cargo fmt --all --check`、`nekonetease-core`/`nekonetease-net`/`nekonetease-app`/`nekonetease-views` 的库测试、`nekonetease` 的 translations 测试，以及 `cargo build -p nekonetease-views`。release build、真实账号探针与 Windows 构建未执行。
- 工具链已改为跟随宿主 stable（本机 1.99.1），MSRV 为 1.99；`cargo clippy --all-targets --locked --workspace -- -D warnings` 可直接在本机复现 CI gate。
- 当前 checkout 不包含 `CLAUDE.md` 引用的 `.claude/rules/` 目录；不要把其中的规则引用当作已交付文件，也不要擅自补写。
- 本轮新增的 msgid（`Playing a preview`、`Membership required`、`Purchase required`、`Playing at a lower quality`、`Lv. {}`、`Charts`、`Collect`/`Collected`、`Move up`/`Move down`）和四个基线遗漏 msgid（`Playlists`、`Not interested`、`Recent comments`、`Sending…`）是手工写入各语言 catalog 和 `.pot` 的，因为当前环境没有 `slint-tr-extractor`。需要在装有该工具的机器上运行 `scripts/update-translations.sh` 重新生成并核对，避免模板与 catalog 漂移。
- `.cargo/config.toml` 仍未纳入版本库；提交时必须与代码一起提交，否则干净 clone 无法编译。
