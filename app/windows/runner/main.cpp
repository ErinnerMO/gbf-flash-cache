#include <flutter/dart_project.h>
#include <flutter/flutter_view_controller.h>
#include <windows.h>
#include <charconv>
#include <filesystem>
#include <functional>
#include <string>

#include "flutter_window.h"
#include "utils.h"

// Opt this process into native dark popup menus on Windows 10 1903+.
// Older systems retain their native menu; never call a mismatched ordinal ABI.
void EnableDarkMenus(HWND window) {
  auto ntdll = GetModuleHandleW(L"ntdll.dll");
  using Version = LONG(WINAPI*)(OSVERSIONINFOW*);
  auto version_fn = reinterpret_cast<Version>(GetProcAddress(ntdll, "RtlGetVersion"));
  OSVERSIONINFOW version{};
  version.dwOSVersionInfoSize = sizeof(version);
  if (!version_fn || version_fn(&version) != 0 || version.dwMajorVersion < 10 || version.dwBuildNumber < 18362) return;
  static auto theme = LoadLibraryExW(L"uxtheme.dll", nullptr, LOAD_LIBRARY_SEARCH_SYSTEM32);
  if (!theme) return;
  using PreferredMode = int(WINAPI*)(int);
  using FlushMenus = void(WINAPI*)();
  auto preferred = reinterpret_cast<PreferredMode>(GetProcAddress(theme, MAKEINTRESOURCEA(135)));
  auto flush = reinterpret_cast<FlushMenus>(GetProcAddress(theme, MAKEINTRESOURCEA(136)));
  using AllowWindow = bool(WINAPI*)(HWND, bool);
  auto allow_window = reinterpret_cast<AllowWindow>(GetProcAddress(theme, MAKEINTRESOURCEA(133)));
  if (allow_window) allow_window(window, true);
  if (preferred) preferred(2);  // ForceDark, scoped to this process.
  if (flush) flush();
  // Keep uxtheme loaded: unloading resets its process-level theme state.
}

int APIENTRY wWinMain(_In_ HINSTANCE instance, _In_opt_ HINSTANCE prev,
                      _In_ wchar_t *command_line, _In_ int show_command) {
  wchar_t executable[32768]{};
  GetModuleFileNameW(nullptr, executable, 32768);
  auto command_line_arguments = GetCommandLineArguments();
  // A restart must release the old process's data locks before opening the core.
  for (const auto& argument : command_line_arguments) {
    const std::string prefix = "--gfc-wait-for=";
    if (argument.rfind(prefix, 0) != 0) continue;
    DWORD parent_pid = 0;
    const auto end = argument.data() + argument.size();
    const auto parsed = std::from_chars(argument.data() + prefix.size(), end, parent_pid);
    if (parsed.ec != std::errc{} || parsed.ptr != end || !parent_pid ||
        parent_pid == GetCurrentProcessId()) return EXIT_FAILURE;
    HANDLE parent = OpenProcess(SYNCHRONIZE, FALSE, parent_pid);
    if (!parent) {
      if (GetLastError() == ERROR_INVALID_PARAMETER) continue;  // Already exited.
      MessageBoxW(nullptr, L"无法等待旧进程退出，请手动重新启动应用。", L"GBF Flash Cache", MB_OK | MB_ICONERROR);
      return EXIT_FAILURE;
    }
    const auto waited = WaitForSingleObject(parent, 30000);
    CloseHandle(parent);
    if (waited != WAIT_OBJECT_0) {
      MessageBoxW(nullptr, L"旧进程尚未退出，请稍后手动重新启动应用。", L"GBF Flash Cache", MB_OK | MB_ICONERROR);
      return EXIT_FAILURE;
    }
  }
  // Original and compatibility EXEs must focus the same instance, not share data concurrently.
  auto directory = std::filesystem::path(executable).parent_path().wstring();
  CharLowerBuffW(directory.data(), static_cast<DWORD>(directory.size()));
  const std::wstring instance_name = L"Local\\GBF-FLASH-CACHE-" +
      std::to_wstring(std::hash<std::wstring>{}(directory));
  HANDLE single = CreateMutexW(nullptr, FALSE, instance_name.c_str());
  if (!single) return EXIT_FAILURE;
  const auto acquired = WaitForSingleObject(single, 0);
  if (acquired != WAIT_OBJECT_0 && acquired != WAIT_ABANDONED) {
    for (int i = 0; i < 40; ++i) {
      struct Search { const wchar_t* name; HWND found; } search{instance_name.c_str(), nullptr};
      EnumWindows([](HWND hwnd, LPARAM param) -> BOOL {
        auto* s = reinterpret_cast<Search*>(param);
        if (GetPropW(hwnd, s->name)) { s->found = hwnd; return FALSE; }
        return TRUE;
      }, reinterpret_cast<LPARAM>(&search));
      if (search.found) {
        DWORD pid = 0;
        GetWindowThreadProcessId(search.found, &pid);
        AllowSetForegroundWindow(pid);
        PostMessageW(search.found, WM_APP + 73, 0, 0);
        break;
      }
      Sleep(50);
    }
    CloseHandle(single);
    return EXIT_SUCCESS;
  }
  // Attach to console when present (e.g., 'flutter run') or create a
  // new console when running with a debugger.
  if (!::AttachConsole(ATTACH_PARENT_PROCESS) && ::IsDebuggerPresent()) {
    CreateAndAttachConsole();
  }

  // Initialize COM, so that it is available for use in the library and/or
  // plugins.
  ::CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);

  flutter::DartProject project(L"ui-assets");

  project.set_dart_entrypoint_arguments(std::move(command_line_arguments));

  FlutterWindow window(project);
  Win32Window::Point origin(10, 10);
  Win32Window::Size size(1280, 720);
  if (!window.Create(L"GBF Flash Cache", origin, size)) {
    return EXIT_FAILURE;
  }
  SetPropW(window.GetHandle(), instance_name.c_str(), reinterpret_cast<HANDLE>(1));
  EnableDarkMenus(window.GetHandle());
  window.SetQuitOnClose(true);

  ::MSG msg;
  while (::GetMessage(&msg, nullptr, 0, 0)) {
    ::TranslateMessage(&msg);
    ::DispatchMessage(&msg);
  }

  if (single) CloseHandle(single);
  ::CoUninitialize();
  return EXIT_SUCCESS;
}
