# NekoNetease

[![License: AGPL-3.0-or-later](https://img.shields.io/badge/License-AGPL--3.0--or--later-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Built%20with-Rust-orange.svg)](https://www.rust-lang.org/)

**NekoNetease 是 NekoRain 基于 [Melodia](https://github.com/KenanSalar/Melodia) 改进的第三方网易云音乐桌面客户端。**它使用 Rust 和 Slint 构建，在 Melodia 的桌面音乐播放器基础上整合了网易云音乐账号、曲库浏览和在线播放功能，同时保留本地音乐管理与播放能力。

> **第三方项目声明**：NekoNetease 由社区独立开发，与网易云音乐及其关联公司无隶属、授权或官方支持关系。网易云音乐的账号、接口、曲库和播放权限由相关服务提供方管理；服务规则或接口变化可能影响部分功能。请遵守相关服务条款并合理使用。

## 功能

- **网易云账号**：支持二维码、手机号和邮箱登录。
- **在线音乐**：歌曲、专辑、歌手和歌单四种搜索模式，支持服务端分页、Top Result 和专辑/歌手/歌单详情跳转；搜索页空态提供热搜、推荐歌单与排行榜，排行榜另有独立页面；可播放网易云音乐曲目，并在设置中选择云端播放音质。
- **个人音乐**：浏览收藏和创建的歌单、喜欢的歌曲、最近播放与每日推荐；自己创建的歌单可新建、改名、删除、拖拽排序，并可编辑描述、标签与封面；云曲目可加入或移出自己创建的歌单，非自己创建的歌单可收藏/取消收藏；私人漫游是独立页面，提供私人 FM、心动与相似歌曲三种模式。
- **MV**：音乐视频独立页面，支持清晰度切换、可拖动进度、全屏与快捷键；可从歌名右侧的图标、歌曲右键或歌手页进入；带点赞、收藏、分享与整页评论。
- **播客与 DJ 电台**：电台推荐、电台详情与订阅、节目列表与节目播放。
- **评论与社交**：云专辑、歌单、歌曲与 MV 均可打开完整评论区（热门/最新、分页、回复、发表、删除、点赞、头像）；用户主页、关注、动态、私信与通知在设置页进入，私信只读且不落库。
- **云盘与离线**：云盘歌曲列表、云盘与本地曲目的对应识别、云端歌单到本地歌单的复核式同步（先看差异再应用），以及有上限与过期的流媒体离线缓存。
- **歌词**：显示在线曲目歌词，也支持本地歌词文件和音乐文件内嵌歌词。
- **本地音乐**：扫描和管理本地曲库，支持搜索、歌单、标签编辑及多种常见音频格式。
- **桌面播放体验**：包含播放队列、均衡器、ReplayGain、无缝播放、迷你播放器、系统媒体按键和多种主题。
- **跨平台**：支持 Linux 与 Windows。当前发布构建覆盖 `x86_64`：Linux 提供 tar.gz、AppImage、RPM 和 DEB，Windows 提供 MSI 与便携 ZIP。`aarch64` 不受支持：它不在发布矩阵中，而且不只是缺两个矩阵槽——Linux 打包步骤与两个 Windows 产物名都写死了 `x86_64`/`amd64`，RPM 路径和 AppImage 工具链也按架构区分，移植需要这些步骤一起改动。

网易云音乐功能需要网络连接。部分歌曲、音质或账号功能可能受账号状态、会员权限、地区及服务端策略限制。

## 下载

前往 [GitHub Releases](https://github.com/HELPMEEADICE/NekoNetease/releases) 下载适用于你系统的版本（当前发布仅提供 `x86_64`）。Linux 提供 `.deb`、`.rpm`、AppImage 或压缩包等构建；Windows 提供 `.msi` 安装包和 `.zip` 便携版（以对应版本的发布附件为准）。

## 从源码构建

项目使用 Rust 2024 edition，跟随宿主 stable 工具链，最低要求 Rust 1.99；配置见 [`rust-toolchain.toml`](rust-toolchain.toml)。Linux 构建还需要 Slint/FemtoVG 所需的系统开发依赖；Windows 无需额外的项目专用依赖。

```bash
git clone https://github.com/HELPMEEADICE/NekoNetease.git
cd NekoNetease
cargo run -p nekonetease
```

构建发布版本：

```bash
cargo build --release -p nekonetease
```

Linux 常见构建依赖：

```bash
# Debian / Ubuntu
sudo apt install libfontconfig1-dev libfreetype6-dev libasound2-dev \
                 libxkbcommon-dev mesa-vulkan-drivers libwayland-dev

# Fedora
sudo dnf install fontconfig-devel freetype-devel alsa-lib-devel \
                 libxkbcommon-devel vulkan-loader wayland-devel
```

## 项目来源与许可

NekoNetease 由 NekoRain 维护，基于 [Kenan Salar 的 Melodia](https://github.com/KenanSalar/Melodia) 改进。感谢 Kenan Salar、Melodia 项目及其贡献者提供的基础播放器实现；Kenan Salar 是参考项目作者，并非本项目作者。

NekoNetease 按 [GNU Affero 通用公共许可证 v3 或更高版本（AGPL-3.0-or-later）](LICENSE) 发布。分发或修改本项目时，请遵守许可证要求；随项目提供的第三方组件许可文本见 [`licenses/`](licenses/)。

网易云音乐及相关商标、服务和内容归其各自权利人所有，本项目不主张相关权利。
