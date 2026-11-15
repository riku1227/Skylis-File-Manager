// The Explorer context command runs in a short-lived, out-of-process STA server.
// No foreground-window guessing, path-only window searches, or resident hooks.
#include <atomic>
#include <chrono>
#include <condition_variable>
#include <exdisp.h>
#include <memory>
#include <mutex>
#include <ocidl.h>
#include <shlobj.h>
#include <shlwapi.h>
#include <shobjidl.h>
#include <string>
#include <thread>
#include <uiautomation.h>
#include <vector>
#include <windows.h>
#include <wrl/client.h>

using Microsoft::WRL::ComPtr;
static const CLSID CLSID_SkylisSwitch = {
    0x8459d65f, 0x75c4, 0x4aa0, {0xb8, 0x73, 0x14, 0x7a, 0x17, 0xe1, 0x2c, 0x60}};
static constexpr DWORD RequiredProofs = (1u << 7) - 1;
extern "C" int skylis_handoff_accept_close(unsigned flags) {
    return (flags & RequiredProofs) == RequiredProofs;
}
template <typename Inspect, typename Close>
static bool recheck_and_close(Inspect inspect, Close close) {
    if (!skylis_handoff_accept_close(inspect()))
        return false;
    // No cached result crosses the verification boundary. Every identity and tab proof
    // is fetched again after the first round of potentially blocking Shell calls.
    if (!skylis_handoff_accept_close(inspect()))
        return false;
    return close();
}
extern "C" unsigned skylis_handoff_test_recheck(unsigned first, unsigned second) {
    unsigned inspected = 0, closed = 0;
    recheck_and_close([&] { return inspected++ == 0 ? first : second; },
                      [&] {
                          ++closed;
                          return true;
                      });
    return inspected | (closed << 8);
}

struct Handle {
    HANDLE value = nullptr;
    Handle() = default;
    explicit Handle(HANDLE h) : value(h) {}
    ~Handle() {
        if (value && value != INVALID_HANDLE_VALUE)
            CloseHandle(value);
    }
    Handle(const Handle &) = delete;
    Handle &operator=(const Handle &) = delete;
    void reset(HANDLE h) {
        if (value && value != INVALID_HANDLE_VALUE)
            CloseHandle(value);
        value = h;
    }
};
struct Pidl {
    PIDLIST_ABSOLUTE value = nullptr;
    ~Pidl() { CoTaskMemFree(value); }
};
// Shell proxies can hang independently of UI Automation's timeout. Request cancellation,
// then bound even an uncooperative proxy by exiting ONLY this disposable helper process.
class CallBudget {
    struct State {
        std::mutex mutex;
        std::condition_variable wake;
        bool finished = false;
        std::atomic<bool> expired{false};
        Handle timeout_event;
    };
    std::shared_ptr<State> state = std::make_shared<State>();
    std::thread watchdog;
    bool cancellation_enabled = false;
    bool valid = false;
    static HANDLE duplicate(HANDLE original) {
        HANDLE copy = nullptr;
        if (original)
            DuplicateHandle(GetCurrentProcess(), original, GetCurrentProcess(), &copy, 0, FALSE,
                            DUPLICATE_SAME_ACCESS);
        return copy;
    }

  public:
    explicit CallBudget(DWORD milliseconds, HANDLE timeout_event = nullptr,
                        bool cancel_calls = true) {
        state->timeout_event.reset(duplicate(timeout_event));
        cancellation_enabled = cancel_calls && SUCCEEDED(CoEnableCallCancellation(nullptr));
        const auto target_thread = GetCurrentThreadId();
        try {
            watchdog = std::thread([shared = state, milliseconds, target_thread] {
                const auto initialized = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
                {
                    std::unique_lock<std::mutex> lock(shared->mutex);
                    if (!shared->wake.wait_for(lock, std::chrono::milliseconds(milliseconds),
                                               [&] { return shared->finished; })) {
                        shared->expired.store(true);
                        if (shared->timeout_event.value)
                            SetEvent(shared->timeout_event.value);
                        lock.unlock();
                        CoCancelCall(target_thread, 0);
                        lock.lock();
                        if (!shared->wake.wait_for(lock, std::chrono::seconds(2),
                                                   [&] { return shared->finished; })) {
                            // Never touch Explorer or the launched GPUI process.
                            TerminateProcess(GetCurrentProcess(), ERROR_TIMEOUT);
                        }
                    }
                }
                if (SUCCEEDED(initialized))
                    CoUninitialize();
            });
            valid = true;
        } catch (...) {
            state->expired.store(true);
        }
    }
    ~CallBudget() {
        {
            std::lock_guard<std::mutex> lock(state->mutex);
            state->finished = true;
        }
        state->wake.notify_one();
        if (watchdog.joinable())
            watchdog.join();
        if (cancellation_enabled)
            CoDisableCallCancellation(nullptr);
    }
    bool available() const { return valid && !state->expired.load(); }
    bool close(HWND window, HANDLE uncertain_event) {
        std::lock_guard<std::mutex> lock(state->mutex);
        if (!valid || state->expired.load())
            return false;
        if (!PostMessageW(window, WM_CLOSE, 0, 0))
            return false;
        // A sent close may complete later. A deadline after this point cannot claim retained.
        state->timeout_event.reset(duplicate(uncertain_event));
        return true;
    }
};
struct Events {
    Handle ready, cancel, closed, retained, uncertain;
};
static std::wstring event_name(const wchar_t *token, const wchar_t *suffix) {
    return std::wstring(L"Local\\Skylis.Handoff.") + token + suffix;
}
static bool valid_token(const wchar_t *token) {
    if (!token || wcsnlen(token, 37) != 36)
        return false;
    for (size_t i = 0; i < 36; ++i) {
        if (i == 8 || i == 13 || i == 18 || i == 23) {
            if (token[i] != L'-')
                return false;
        } else if (!((token[i] >= L'0' && token[i] <= L'9') ||
                     (token[i] >= L'a' && token[i] <= L'f') ||
                     (token[i] >= L'A' && token[i] <= L'F')))
            return false;
    }
    return true;
}
static bool events(Events &e, const wchar_t *token, bool create) {
    struct Item {
        Handle *h;
        const wchar_t *suffix;
    };
    for (auto item :
         {Item{&e.ready, L".ready"}, Item{&e.cancel, L".cancel"}, Item{&e.closed, L".closed"},
          Item{&e.retained, L".retained"}, Item{&e.uncertain, L".uncertain"}}) {
        auto name = event_name(token, item.suffix);
        SetLastError(ERROR_SUCCESS);
        HANDLE h = create ? CreateEventW(nullptr, TRUE, FALSE, name.c_str())
                          : OpenEventW(SYNCHRONIZE | EVENT_MODIFY_STATE, FALSE, name.c_str());
        const auto error = GetLastError();
        item.h->reset(h);
        if (!h || (create && error == ERROR_ALREADY_EXISTS))
            return false;
    }
    return true;
}
extern "C" void *skylis_handoff_open(const wchar_t *token) {
    if (!valid_token(token))
        return nullptr;
    auto e = std::make_unique<Events>();
    if (!events(*e, token, false))
        return nullptr;
    return e.release();
}
extern "C" int skylis_handoff_complete_wait(void *opaque) {
    std::unique_ptr<Events> e(static_cast<Events *>(opaque));
    if (!e || WaitForSingleObject(e->cancel.value, 0) == WAIT_OBJECT_0 || !SetEvent(e->ready.value))
        return 2;
    HANDLE handles[] = {e->cancel.value, e->closed.value, e->retained.value, e->uncertain.value};
    auto result = WaitForMultipleObjects(4, handles, FALSE, 15000);
    if (result == WAIT_OBJECT_0 + 1)
        return 1;
    if (result == WAIT_OBJECT_0 || result == WAIT_OBJECT_0 + 2)
        return 2;
    return 3;
}
extern "C" void *skylis_handoff_duplicate(void *opaque) {
    auto original = static_cast<Events *>(opaque);
    if (!original)
        return nullptr;
    auto copy = std::make_unique<Events>();
    struct Pair {
        Handle *from;
        Handle *to;
    };
    for (auto pair :
         {Pair{&original->ready, &copy->ready}, Pair{&original->cancel, &copy->cancel},
          Pair{&original->closed, &copy->closed}, Pair{&original->retained, &copy->retained},
          Pair{&original->uncertain, &copy->uncertain}}) {
        HANDLE duplicate = nullptr;
        if (!DuplicateHandle(GetCurrentProcess(), pair.from->value, GetCurrentProcess(), &duplicate,
                             0, FALSE, DUPLICATE_SAME_ACCESS))
            return nullptr;
        pair.to->reset(duplicate);
    }
    return copy.release();
}
extern "C" void skylis_handoff_cancel(void *opaque) {
    std::unique_ptr<Events> e(static_cast<Events *>(opaque));
    if (e)
        SetEvent(e->cancel.value);
}

static std::wstring quote(const std::wstring &value) {
    std::wstring out = L"\"";
    size_t slashes = 0;
    for (wchar_t c : value) {
        if (c == L'\\') {
            ++slashes;
            continue;
        }
        out.append(slashes * (c == L'\"' ? 2 : 1), L'\\');
        slashes = 0;
        if (c == L'\"')
            out.push_back(L'\\');
        out.push_back(c);
    }
    out.append(slashes * 2, L'\\');
    out.push_back(L'\"');
    return out;
}
static std::wstring exe_path() {
    std::vector<wchar_t> buf(32768);
    DWORD len = GetModuleFileNameW(nullptr, buf.data(), static_cast<DWORD>(buf.size()));
    return len && len < buf.size() ? std::wstring(buf.data(), len) : std::wstring();
}
static bool launch(const std::wstring &folder, const std::wstring &token, Handle &process) {
    const auto exe = exe_path();
    if (exe.empty() || folder.empty())
        return false;
    std::wstring cmd =
        quote(exe) + (token.empty() ? L" --switch-explorer " : L" --path ") + quote(folder);
    if (!token.empty())
        cmd += L" --handoff-token " + token;
    STARTUPINFOW startup{};
    startup.cb = sizeof(startup);
    PROCESS_INFORMATION child{};
    if (!CreateProcessW(exe.c_str(), cmd.data(), nullptr, nullptr, FALSE, 0, nullptr, nullptr,
                        &startup, &child))
        return false;
    process.reset(child.hProcess);
    AllowSetForegroundWindow(child.dwProcessId);
    CloseHandle(child.hThread);
    return true;
}
static bool view_folder(IShellView *view, Pidl &pidl, std::wstring &path) {
    ComPtr<IFolderView> folder;
    ComPtr<IPersistFolder2> persist;
    if (!view || FAILED(view->QueryInterface(IID_PPV_ARGS(&folder))) ||
        FAILED(folder->GetFolder(IID_PPV_ARGS(&persist))) ||
        FAILED(persist->GetCurFolder(&pidl.value)))
        return false;
    ComPtr<IShellItem> item;
    if (FAILED(SHCreateItemFromIDList(pidl.value, IID_PPV_ARGS(&item))))
        return false;
    PWSTR raw = nullptr;
    if (FAILED(item->GetDisplayName(SIGDN_FILESYSPATH, &raw)))
        return false;
    path = raw;
    CoTaskMemFree(raw);
    return !path.empty();
}
static bool process_identity(HWND hwnd, DWORD &pid, FILETIME &creation) {
    if (!IsWindow(hwnd) || !GetWindowThreadProcessId(hwnd, &pid))
        return false;
    Handle process(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid));
    if (!process.value)
        return false;
    wchar_t image[32768];
    DWORD len = ARRAYSIZE(image);
    if (!QueryFullProcessImageNameW(process.value, 0, image, &len))
        return false;
    wchar_t windows[32768];
    UINT count = GetWindowsDirectoryW(windows, ARRAYSIZE(windows));
    if (!count || count >= ARRAYSIZE(windows))
        return false;
    auto expected = std::wstring(windows, count) + L"\\explorer.exe";
    if (_wcsicmp(image, expected.c_str()) != 0)
        return false;
    FILETIME exit{}, kernel{}, user{};
    return GetProcessTimes(process.value, &creation, &exit, &kernel, &user) != FALSE;
}

struct Origin {
    ComPtr<IShellBrowser> browser;
    ComPtr<IUnknown> view_identity;
    HWND root = nullptr;
    DWORD pid = 0;
    FILETIME creation{};
    Pidl folder;
    std::wstring path;
    bool capture(IUnknown *site) {
        ComPtr<IServiceProvider> provider;
        ComPtr<IShellView> source;
        if (!site || FAILED(site->QueryInterface(IID_PPV_ARGS(&provider))) ||
            FAILED(provider->QueryService(SID_SFolderView, IID_PPV_ARGS(&source))) ||
            !view_folder(source.Get(), folder, path) ||
            FAILED(source->QueryInterface(IID_PPV_ARGS(&view_identity))) ||
            FAILED(provider->QueryService(SID_STopLevelBrowser, IID_PPV_ARGS(&browser))))
            return false;
        // The site identifies the invocation's view. The currently active browser view
        // is only a cross-check, never a replacement source if focus/tab changed.
        ComPtr<IShellView> view;
        ComPtr<IUnknown> active_identity;
        HWND hwnd = nullptr, source_hwnd = nullptr;
        if (FAILED(browser->GetWindow(&hwnd)) || FAILED(browser->QueryActiveShellView(&view)) ||
            FAILED(view->QueryInterface(IID_PPV_ARGS(&active_identity))) ||
            active_identity.Get() != view_identity.Get() || FAILED(source->GetWindow(&source_hwnd)))
            return false;
        root = GetAncestor(hwnd, GA_ROOT);
        wchar_t cls[64]{};
        if (!root || GetAncestor(source_hwnd, GA_ROOT) != root ||
            !GetClassNameW(root, cls, ARRAYSIZE(cls)) || wcscmp(cls, L"CabinetWClass") != 0)
            return false;
        return process_identity(root, pid, creation);
    }
    bool same_process() const {
        DWORD current_pid = 0;
        FILETIME current_creation{};
        return process_identity(root, current_pid, current_creation) && current_pid == pid &&
               CompareFileTime(&creation, &current_creation) == 0;
    }
    bool same_view(bool &same_folder) const {
        same_folder = false;
        ComPtr<IShellView> view;
        ComPtr<IUnknown> identity;
        HWND hwnd = nullptr;
        if (!browser || FAILED(browser->GetWindow(&hwnd)) || GetAncestor(hwnd, GA_ROOT) != root ||
            FAILED(browser->QueryActiveShellView(&view)) ||
            FAILED(view->QueryInterface(IID_PPV_ARGS(&identity))) ||
            identity.Get() != view_identity.Get())
            return false;
        Pidl now;
        std::wstring path_now;
        same_folder = view_folder(view.Get(), now, path_now) && ILIsEqual(folder.value, now.value);
        return true;
    }
};

// Require two positive tab-count observations. Unknown legacy/changed UI retains Explorer.
// Count all tabs, including offscreen tabs. Never infer single-tab from foreground state.
static bool single_tab(HWND root) {
    struct Count {
        unsigned tabs = 0;
    } count;
    EnumChildWindows(
        root,
        [](HWND child, LPARAM data) -> BOOL {
            wchar_t cls[128]{};
            if (GetClassNameW(child, cls, ARRAYSIZE(cls)) &&
                wcscmp(cls, L"ShellTabWindowClass") == 0)
                ++reinterpret_cast<Count *>(data)->tabs;
            return TRUE;
        },
        reinterpret_cast<LPARAM>(&count));
    if (count.tabs != 1)
        return false;
    ComPtr<IUIAutomation> automation;
    ComPtr<IUIAutomationElement> element;
    ComPtr<IUIAutomationCondition> condition;
    ComPtr<IUIAutomationElementArray> matches;
    ComPtr<IUIAutomation2> bounded;
    if (FAILED(CoCreateInstance(CLSID_CUIAutomation8, nullptr, CLSCTX_INPROC_SERVER,
                                IID_PPV_ARGS(&automation))) ||
        FAILED(automation.As(&bounded)) || FAILED(bounded->put_ConnectionTimeout(1000)) ||
        FAILED(bounded->put_TransactionTimeout(1000)) ||
        FAILED(automation->ElementFromHandle(root, &element)))
        return false;
    VARIANT type;
    VariantInit(&type);
    type.vt = VT_I4;
    type.lVal = UIA_TabItemControlTypeId;
    if (FAILED(automation->CreatePropertyCondition(UIA_ControlTypePropertyId, type, &condition)) ||
        FAILED(element->FindAll(TreeScope_Descendants, condition.Get(), &matches)))
        return false;
    int length = 0;
    return SUCCEEDED(matches->get_Length(&length)) && length == 1;
}

struct Request {
    std::unique_ptr<Origin> origin;
    Events events;
    Handle child;
    ULONGLONG started = 0;
};
static std::unique_ptr<Request> pending;
static bool invoked = false;
static bool done = false;

class Command final : public IExecuteCommand,
                      public IObjectWithSelection,
                      public IObjectWithSite,
                      public IInitializeCommand {
    std::atomic<ULONG> refs{1};
    ComPtr<IUnknown> site;
    ComPtr<IShellItemArray> selection;

  public:
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID iid, void **out) override {
        if (!out)
            return E_POINTER;
        *out = nullptr;
        if (iid == IID_IUnknown || iid == __uuidof(IExecuteCommand))
            *out = static_cast<IExecuteCommand *>(this);
        else if (iid == __uuidof(IObjectWithSelection))
            *out = static_cast<IObjectWithSelection *>(this);
        else if (iid == IID_IObjectWithSite)
            *out = static_cast<IObjectWithSite *>(this);
        else if (iid == __uuidof(IInitializeCommand))
            *out = static_cast<IInitializeCommand *>(this);
        else
            return E_NOINTERFACE;
        AddRef();
        return S_OK;
    }
    ULONG STDMETHODCALLTYPE AddRef() override { return ++refs; }
    ULONG STDMETHODCALLTYPE Release() override {
        auto n = --refs;
        if (!n)
            delete this;
        return n;
    }
    HRESULT STDMETHODCALLTYPE Initialize(PCWSTR, IPropertyBag *) override { return S_OK; }
    HRESULT STDMETHODCALLTYPE SetSite(IUnknown *value) override {
        site = value;
        return S_OK;
    }
    HRESULT STDMETHODCALLTYPE GetSite(REFIID iid, void **value) override {
        if (!value)
            return E_POINTER;
        *value = nullptr;
        return site ? site->QueryInterface(iid, value) : E_FAIL;
    }
    HRESULT STDMETHODCALLTYPE SetSelection(IShellItemArray *value) override {
        selection = value;
        return S_OK;
    }
    HRESULT STDMETHODCALLTYPE GetSelection(REFIID iid, void **value) override {
        if (!value)
            return E_POINTER;
        *value = nullptr;
        return selection ? selection->QueryInterface(iid, value) : E_FAIL;
    }
    HRESULT STDMETHODCALLTYPE SetKeyState(DWORD) override { return S_OK; }
    HRESULT STDMETHODCALLTYPE SetParameters(PCWSTR) override { return S_OK; }
    HRESULT STDMETHODCALLTYPE SetPosition(POINT) override { return S_OK; }
    HRESULT STDMETHODCALLTYPE SetShowWindow(int) override { return S_OK; }
    HRESULT STDMETHODCALLTYPE SetNoShowUI(BOOL) override { return S_OK; }
    HRESULT STDMETHODCALLTYPE SetDirectory(PCWSTR) override { return S_OK; }
    HRESULT STDMETHODCALLTYPE Execute() override {
        if (invoked)
            return HRESULT_FROM_WIN32(ERROR_BUSY);
        invoked = true;
        CallBudget budget(6000);
        if (!budget.available()) {
            done = true;
            return HRESULT_FROM_WIN32(ERROR_TIMEOUT);
        }
        auto request = std::make_unique<Request>();
        request->origin = std::make_unique<Origin>();
        const bool trusted = request->origin->capture(site.Get());
        std::wstring path = request->origin->path;
        if (path.empty() && selection) {
            DWORD count = 0;
            ComPtr<IShellItem> item;
            SFGAOF attributes = 0;
            PWSTR raw = nullptr;
            if (SUCCEEDED(selection->GetCount(&count)) && count == 1 &&
                SUCCEEDED(selection->GetItemAt(0, &item)) &&
                SUCCEEDED(item->GetAttributes(SFGAO_FOLDER | SFGAO_FILESYSTEM, &attributes)) &&
                (attributes & (SFGAO_FOLDER | SFGAO_FILESYSTEM)) ==
                    (SFGAO_FOLDER | SFGAO_FILESYSTEM) &&
                SUCCEEDED(item->GetDisplayName(SIGDN_FILESYSPATH, &raw))) {
                path = raw;
                CoTaskMemFree(raw);
            }
        }
        if (path.empty()) {
            done = true;
            return HRESULT_FROM_WIN32(ERROR_NOT_SUPPORTED);
        }
        GUID guid{};
        wchar_t text[40]{};
        std::wstring token;
        if (trusted && SUCCEEDED(CoCreateGuid(&guid)) &&
            StringFromGUID2(guid, text, ARRAYSIZE(text))) {
            token.assign(text + 1, 36);
            if (!events(request->events, token.c_str(), true))
                token.clear();
        }
        if (!budget.available()) {
            done = true;
            return HRESULT_FROM_WIN32(ERROR_TIMEOUT);
        }
        if (!launch(path, token, request->child)) {
            done = true;
            return HRESULT_FROM_WIN32(GetLastError());
        }
        if (token.empty()) {
            done = true;
            return S_OK;
        }
        request->started = GetTickCount64();
        pending = std::move(request);
        return S_OK;
    }
};
class Factory final : public IClassFactory {
    std::atomic<ULONG> refs{1};

  public:
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID iid, void **out) override {
        if (!out)
            return E_POINTER;
        *out = nullptr;
        if (iid != IID_IUnknown && iid != IID_IClassFactory)
            return E_NOINTERFACE;
        *out = static_cast<IClassFactory *>(this);
        AddRef();
        return S_OK;
    }
    ULONG STDMETHODCALLTYPE AddRef() override { return ++refs; }
    ULONG STDMETHODCALLTYPE Release() override {
        auto n = --refs;
        if (!n)
            delete this;
        return n;
    }
    HRESULT STDMETHODCALLTYPE CreateInstance(IUnknown *outer, REFIID iid, void **out) override {
        if (outer)
            return CLASS_E_NOAGGREGATION;
        auto command = new (std::nothrow) Command;
        if (!command)
            return E_OUTOFMEMORY;
        HRESULT result = command->QueryInterface(iid, out);
        command->Release();
        return result;
    }
    HRESULT STDMETHODCALLTYPE LockServer(BOOL) override { return S_OK; }
};
static void pump() {
    MSG msg;
    while (PeekMessageW(&msg, nullptr, 0, 0, PM_REMOVE)) {
        if (msg.message == WM_QUIT) {
            done = true;
            break;
        }
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}
static bool alive(HANDLE process) { return WaitForSingleObject(process, 0) == WAIT_TIMEOUT; }
static void finish_request() {
    auto &r = *pending;
    CallBudget budget(6000, r.events.retained.value);
    if (!budget.available()) {
        SetEvent(r.events.retained.value);
        done = true;
        return;
    }
    auto inspect = [&] {
        if (!budget.available())
            return DWORD(0);
        bool folder_unchanged = false;
        const bool view_unchanged = r.origin->same_view(folder_unchanged);
        DWORD flags = 0;
        if (WaitForSingleObject(r.events.ready.value, 0) == WAIT_OBJECT_0 &&
            WaitForSingleObject(r.events.cancel.value, 0) != WAIT_OBJECT_0)
            flags |= 1u;
        if (r.origin->browser && r.origin->view_identity)
            flags |= 1u << 1;
        if (r.origin->same_process())
            flags |= 1u << 2;
        if (folder_unchanged)
            flags |= 1u << 3;
        if (single_tab(r.origin->root))
            flags |= 1u << 4;
        if (view_unchanged)
            flags |= 1u << 5;
        if (alive(r.child.value))
            flags |= 1u << 6;
        return flags;
    };
    bool closed = false;
    const bool close_requested = recheck_and_close(inspect, [&] {
        return WaitForSingleObject(r.events.cancel.value, 0) != WAIT_OBJECT_0 &&
               alive(r.child.value) && budget.close(r.origin->root, r.events.uncertain.value);
    });
    if (close_requested) {
        const auto until = GetTickCount64() + 1500;
        do {
            if (!IsWindow(r.origin->root)) {
                closed = true;
                break;
            }
            MsgWaitForMultipleObjectsEx(0, nullptr, 20, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
            pump();
        } while (GetTickCount64() < until);
    }
    SetEvent(closed            ? r.events.closed.value
             : close_requested ? r.events.uncertain.value
                               : r.events.retained.value);
    done = true;
}
extern "C" void skylis_run_shell_command_server() {
    if (FAILED(CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE)))
        return;
    // Also bound apartment teardown/Release calls outside an individual command budget.
    CallBudget lifetime(120000, nullptr, false);
    if (!lifetime.available()) {
        CoUninitialize();
        return;
    }
    auto factory = new (std::nothrow) Factory;
    DWORD cookie = 0;
    if (factory && SUCCEEDED(CoRegisterClassObject(CLSID_SkylisSwitch, factory, CLSCTX_LOCAL_SERVER,
                                                   REGCLS_SINGLEUSE | REGCLS_SUSPENDED, &cookie))) {
        CoResumeClassObjects();
        const auto started = GetTickCount64();
        while (!done && GetTickCount64() - started < 120000) {
            pump();
            if (done)
                break;
            if (pending) {
                HANDLE handles[] = {pending->events.ready.value, pending->events.cancel.value,
                                    pending->child.value};
                auto result =
                    MsgWaitForMultipleObjectsEx(3, handles, 100, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
                if (result == WAIT_OBJECT_0)
                    finish_request();
                else if (result == WAIT_OBJECT_0 + 1 || result == WAIT_OBJECT_0 + 2 ||
                         GetTickCount64() - pending->started > 90000)
                    done = true;
            } else {
                if (GetTickCount64() - started > 30000)
                    break;
                MsgWaitForMultipleObjectsEx(0, nullptr, 100, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
            }
        }
        if (pending && !done)
            SetEvent(pending->events.retained.value);
        pending.reset();
        CoRevokeClassObject(cookie);
    }
    if (factory)
        factory->Release();
    CoUninitialize();
}
