# 构建与验证

命令默认从仓库根目录执行。当前产品平台仅 Windows、Android；Linux 可用于核心和 Dart ABI 验证，不构建 Linux 应用。不需要 Android 模拟器。

## 本地检查

需要 Rust、Flutter 与 Python 3。已验证的开发工具版本为 Rust 1.98.1、Flutter 3.47.4；原生依赖和 Dart 依赖分别由锁文件固定。

```sh
cargo test --workspace --locked --all-targets -- --skip windows_trust_is_exact_and_old_ca_is_removed
cargo build -p gbf-flash-cache-app --locked
cd app
flutter pub get
flutter analyze
flutter test
dart run tool/native_smoke.dart ../target/debug/libgbf_flash_cache_app.so
```

最后一条是 Linux 本地 ABI 检查，使用临时数据和回环监听。Windows 可传入相应 DLL 路径。

Windows 的 `windows_trust_is_exact_and_old_ca_is_removed` 测试会临时安装并移除随机生成的测试 CA，应在可进行证书测试的环境运行。Windows 打包脚本跳过这一交互测试，需要时可单独执行：

```powershell
cargo test -p gbf-flash-cache-app --locked --test certificates windows_trust_is_exact_and_old_ca_is_removed -- --exact
```

## Windows

需要 Windows、Flutter、Rust 1.98.1、Visual Studio 2022 C++ 工具与 Windows SDK，以及 Python 3。脚本支持现有 `C:\flutter` 和 `C:\BuildTools` 布局；其它安装位置请通过 PATH 和 `GBF_VC_REDIST` 指定。

```powershell
python scripts/collect_licenses.py dist/licenses x86_64-pc-windows-msvc
powershell -ExecutionPolicy Bypass -File scripts/build_windows.ps1
```

默认输出到 `dist/`；可传入 `-OutputDirectory`，或通过构建工作进程提供 `WINDOWS_BUILD_OUTPUT`。可用 `CARGO_TARGET_DIR` 复用绝对路径下的 Rust 构建缓存。

许可收集每次重建指定的专用输出目录，请勿将该目录用于存放其它文件。

脚本执行 Rust 测试、Flutter 检查和测试、原生 ABI 检查，再打包 ZIP 和 SHA256。包中包含应用原生 DLL（包含共享核心）、VC 运行库、MIT 许可证、原生组件声明及 Flutter 自带的许可文件；不包含用户数据和独立 README。

使用远端 Windows 构建工作进程时，应按相同目录结构提供源码及 `dist/licenses/`，由工作进程的入口调用 `scripts/build_windows.ps1`。不要将脚本单独搬到根目录。

发行构建对 Rust 的源码和用户目录进行路径重映射；Flutter 自动生成的插件注册文件使用包 URI，调试信息单独输出到 `app/build/symbols/`，不放入发行包。保留该目录用于对应产物的故障定位，发布前仍需检查最终二进制是否残留内部路径。

## Android

当前脚本运行于 Linux，需要 Java 21、Android SDK（API 36）、NDK 28.2.13676358、Flutter、Rust Android 目标与 Python 3。

```sh
rustup target add aarch64-linux-android x86_64-linux-android
export GBF_ANDROID_SDK=/path/to/android-sdk
export FLUTTER=/path/to/flutter/bin/flutter
python3 scripts/build_android.py
```

未设置 `GBF_ANDROID_SDK` 时兼容现有 `/opt/coder-cache/android-sdk`；`ANDROID_NDK_HOME` 可覆盖 NDK 路径。未设置 `FLUTTER` 时从 PATH 查找。

脚本构建 Rust 应用原生库与核心，收集 ARM64、x86_64 依赖许可的并集，再生成两种架构的签名 APK。产物位于 `app/build/app/outputs/flutter-apk/`。包含 Android 独立系统代理模块。

构建与控件测试不代表 Android 实机证书安装和后台保活已验证。

签名密钥默认位于 `~/.local/share/gbf-flash-cache/android-signing/`，可用 `GBF_ANDROID_KEY_DIR` 指定。首次生成后应妥善保留，后续升级沿用；私钥和密码不得纳入源码或分发包。

AGP 9.1.0 会引入 Kotlin 构建依赖；即使宿主代码为 Java，也需在 settings 中固定兼容 Flutter 的版本（当前 2.2.21），否则完整 APK 构建会被版本检查拒绝。

仅构建原生部分可运行 `scripts/build_native_android.py`。应用完整构建入口负责设置它们需要的环境变量。

系统代理范围逻辑可通过 `python3 scripts/check_android_proxy.py` 在普通 JVM 验证，无需模拟器。Rust 测试包含虚拟 DNS 数据包及本地 CONNECT 转发，Flutter 测试覆盖名单保存和取消授权。以上均不能替代手机上的授权、路由、后台与撤销验证。

### 签名与发行核验

Android 只有使用相同签名才能覆盖升级。更换签名需先卸载旧包，会删除本地数据及 CA 私钥，重装后需安装新的 CA。

发行前核对 APK 签名延续性、原生库与源码对应关系、许可清单及敏感文件夹带，并保存产物哈希与检查结果。
