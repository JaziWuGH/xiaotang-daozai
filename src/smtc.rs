//! 系统媒体会话（SMTC）读取：Windows 10/11 的“正在播放”。
//! 与原工具使用同一套 API（GlobalSystemMediaTransportControlsSessionManager）。

#[cfg(windows)]
#[path="netease_timeline.rs"]
mod netease_timeline;
#[cfg(windows)]
#[path="netease_playback.rs"]
mod netease_playback;

use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MediaKind {
    Music,
    Video,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum PlayMode {
    #[default]
    Unknown,
    Random,
    Order,
    Heart,
    Cycle,
    OneCycle,
}

impl PlayMode {
    pub fn label(self) -> &'static str {
        match self {
            PlayMode::Unknown => "播放模式",
            PlayMode::Random => "随机播放",
            PlayMode::Order => "顺序播放",
            PlayMode::Heart => "心动模式",
            PlayMode::Cycle => "列表循环",
            PlayMode::OneCycle => "单曲循环",
        }
    }
    fn from_u8(value: u8) -> Self {
        match value {
            1 => PlayMode::Random,
            2 => PlayMode::Order,
            3 => PlayMode::Heart,
            4 => PlayMode::Cycle,
            5 => PlayMode::OneCycle,
            _ => PlayMode::Unknown,
        }
    }
}

/// 最近一次读到的播放模式（目前只有网易云能读）。
static PLAY_MODE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
static MODE_ERROR: Mutex<Option<String>> = Mutex::new(None);
/// 界面侧已知的总时长（毫秒），给「按秒定位」在日志还没读到元数据时兜底。
static KNOWN_DURATION_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn play_mode() -> PlayMode {
    PlayMode::from_u8(PLAY_MODE.load(std::sync::atomic::Ordering::Relaxed))
}

pub fn set_play_mode(mode: PlayMode) {
    PLAY_MODE.store(mode as u8, std::sync::atomic::Ordering::Relaxed);
}

pub fn set_mode_error(message: impl Into<String>) {
    if let Ok(mut slot) = MODE_ERROR.lock() {
        *slot = Some(message.into());
    }
}

/// 取走一次待提示的模式切换错误，避免重复弹提示。
pub fn take_mode_error() -> Option<String> {
    MODE_ERROR.lock().ok().and_then(|mut slot| slot.take())
}

#[derive(Clone, Debug, Default)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub source: String,
    pub song_id: Option<u64>,
    pub playing: bool,
    pub position: f64,
    /// 播放位置的采样时刻，避免封面读取和线程交接消耗歌词提前量。
    pub sampled_at: Option<std::time::Instant>,
    pub duration: f64,
    pub artwork: Option<Arc<eframe::egui::ColorImage>>,
    pub playback_type: MediaKind,
    /// 播放模式；只有能读到的客户端（网易云）才不是 Unknown。
    pub play_mode: PlayMode,
}

impl NowPlaying {
    pub fn kind(&self)->MediaKind {
        if self.title.trim().is_empty(){return MediaKind::Unknown;}
        if self.playback_type==MediaKind::Video {return MediaKind::Video;}
        // 部分浏览器把所有媒体都上报为音乐，补充识别明确的视频页面标题。
        let title=self.title.trim().to_lowercase();
        if [" - 抖音"," - youtube","_哔哩哔哩_bilibili"," - 哔哩哔哩"].iter().any(|suffix|title.ends_with(suffix)) {
            return MediaKind::Video;
        }
        let source=self.source.to_lowercase();
        if ["chrome","msedge","firefox","brave","opera"].iter().any(|name|source.contains(name)) {
            let album=self.album.trim().to_lowercase();
            if ["youtube","bilibili","抖音","哔哩哔哩"].contains(&album.as_str()){return MediaKind::Video;}
            // 视频作者也可能被放进 Artist；浏览器缺少专辑信息时不自动认作歌曲。
            if album.is_empty(){return MediaKind::Unknown;}
        }
        if self.artist.trim().is_empty(){MediaKind::Unknown}else{MediaKind::Music}
    }

    pub fn is_music(&self)->bool {self.kind()==MediaKind::Music}
}

#[derive(Default)]
struct ArtworkCache {
    key: String,
    bytes: Vec<u8>,
    image: Option<Arc<eframe::egui::ColorImage>>,
    pending: Option<std::sync::mpsc::Receiver<(String,Option<Vec<u8>>)>>,
}

impl ArtworkCache {
    fn poll(&mut self,key:String,load:impl FnOnce()->Option<Vec<u8>>+Send+'static)->Option<Arc<eframe::egui::ColorImage>> {
        self.get(key.clone(),||None);
        if let Some(rx)=&self.pending {
            match rx.try_recv() {
                Ok((loaded,bytes))=>{
                    self.pending=None;
                    if loaded==key {self.get(key.clone(),||bytes);}
                },
                Err(std::sync::mpsc::TryRecvError::Disconnected)=>self.pending=None,
                Err(std::sync::mpsc::TryRecvError::Empty)=>{},
            }
        }
        // 最多一个封面读取任务；旧任务结束后只采纳当前歌曲的结果。
        if self.pending.is_none() {
            let (tx,rx)=std::sync::mpsc::channel();self.pending=Some(rx);
            std::thread::spawn(move ||{
                #[cfg(windows)]
                let cookie=unsafe {windows::Win32::System::Com::CoIncrementMTAUsage().ok()};
                let _=tx.send((key,load()));
                #[cfg(windows)]
                if let Some(cookie)=cookie {let _=unsafe {windows::Win32::System::Com::CoDecrementMTAUsage(cookie)};}
            });
        }
        self.image.clone()
    }
    fn get(&mut self, key:String, load:impl FnOnce()->Option<Vec<u8>>)->Option<Arc<eframe::egui::ColorImage>> {
        if self.key!=key {self.key=key;self.bytes.clear();self.image=None;}
        // 封面可能晚于歌名更新；每次检查内容，只在图片变化时重新解码。
        if let Some(bytes)=load() {
            if bytes!=self.bytes {
                if let Some(image)=decode_artwork(&bytes) {
                    if self.image.as_deref()!=Some(&image){self.image=Some(Arc::new(image));}
                }
                self.bytes=bytes;
            }
        }
        self.image.clone()
    }
}

fn decode_artwork(bytes:&[u8])->Option<eframe::egui::ColorImage> {
    let image=image::load_from_memory(bytes).ok()?.thumbnail(256,256).to_rgba8();
    Some(eframe::egui::ColorImage::from_rgba_unmultiplied([image.width() as usize,image.height() as usize],image.as_raw()))
}

#[cfg(windows)]
fn media_result<T:windows::core::RuntimeType+'static>(operation:windows::Foundation::IAsyncOperation<T>)->Option<T>{
    let start=std::time::Instant::now();
    while operation.Status().ok()?==windows::Foundation::AsyncStatus::Started{
        if start.elapsed()>=Duration::from_millis(500){let _=operation.Cancel();return None;}
        std::thread::sleep(Duration::from_millis(10));
    }
    operation.GetResults().ok()
}

#[cfg(windows)]
fn read_artwork(props:&windows::Media::Control::GlobalSystemMediaTransportControlsSessionMediaProperties)->Option<Vec<u8>> {
    use windows::core::Interface;
    use windows::Storage::Streams::DataReader;
    let stream=media_result(props.Thumbnail().ok()?.OpenReadAsync().ok()?)?;
    let size=stream.Size().ok()?;
    if size==0 || size>8*1024*1024 {return None;}
    let reader=DataReader::CreateDataReader(&stream.GetInputStreamAt(0).ok()?).ok()?;
    let count=media_result::<u32>(reader.LoadAsync(size as u32).ok()?.cast().ok()?)?;
    if count!=size as u32 {return None;}
    let mut bytes=vec![0;count as usize];
    reader.ReadBytes(&mut bytes).ok()?;
    Some(bytes)
}

pub type Shared = Arc<Mutex<NowPlaying>>;

pub fn shared() -> Shared {
    Arc::new(Mutex::new(NowPlaying::default()))
}

fn choose_media(system:Option<NowPlaying>,native:Option<NowPlaying>)->Option<NowPlaying>{
    if let (Some(system),Some(native))=(&system,&native){
        if system.source.eq_ignore_ascii_case("cloudmusic.exe"){
            let mut media=native.clone();
            if system.title==media.title && system.artist==media.artist && system.artwork.is_some(){media.artwork=system.artwork.clone();}
            return Some(media);
        }
    }
    if system.as_ref().is_some_and(|m|m.playing && m.is_music()){system}
    else if native.as_ref().is_some_and(|m|m.playing){native}
    else{system.or(native)}
}

fn choose_media_for_source(system:Option<NowPlaying>,native:Option<NowPlaying>,previous:&str,native_pending:bool)->Option<NowPlaying>{
    // 暂停或载入下一首时保留控制对象，其他客户端开始播放则照常切换。
    if previous.eq_ignore_ascii_case("cloudmusic.exe") && !system.as_ref().is_some_and(|m|m.playing)
        && (native.is_some() || native_pending) {return native;}
    choose_media(system,native)
}

#[cfg(windows)]
fn query(artwork:&mut ArtworkCache) -> Option<NowPlaying> {
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager;

    let mgr=media_result(GlobalSystemMediaTransportControlsSessionManager::RequestAsync().ok()?)?;
    let current=mgr.GetCurrentSession().ok();
    let mut sessions=current.iter().cloned().collect::<Vec<_>>();
    if let Ok(all)=mgr.GetSessions(){for session in all{if !sessions.contains(&session){sessions.push(session);}}}
    // 关闭播放器后 Windows 可能仍保留旧的当前会话，优先接入实际在播放的会话。
    for session in &sessions{
        if session.GetPlaybackInfo().ok().and_then(|info|info.PlaybackStatus().ok())==Some(windows::Media::Control::GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing){
            if let Some(media)=read_session(session,artwork){return Some(media);}
        }
    }
    current.as_ref().and_then(|session|read_session(session,artwork))
}

#[cfg(windows)]
fn read_session(session:&windows::Media::Control::GlobalSystemMediaTransportControlsSession,artwork:&mut ArtworkCache)->Option<NowPlaying>{
    let props=media_result(session.TryGetMediaPropertiesAsync().ok()?)?;
    let title = props.Title().map(|s| s.to_string()).unwrap_or_default();
    let artist = props.Artist().map(|s| s.to_string()).unwrap_or_default();
    if title.is_empty() {
        return None;
    }
    let mut playing = false;
    let mut position = 0.0;
    let mut duration = 0.0;
    let mut sampled_at=None;
    let mut playback_type=props.PlaybackType().ok().and_then(|kind|kind.Value().ok());
    if let Ok(info) = session.GetPlaybackInfo() {
        let info_type=info.PlaybackType().ok().and_then(|kind|kind.Value().ok());
        if playback_type.is_none() || info_type.is_some_and(|kind|kind.0==2){playback_type=info_type;}
        if let Ok(status) = info.PlaybackStatus() {
            playing = status == windows::Media::Control::GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing;
        }
    }
    if let Ok(tl) = session.GetTimelineProperties() {
        sampled_at=Some(std::time::Instant::now());
        if let Ok(p) = tl.Position() {
            position = p.Duration as f64 / 10_000_000.0;
        }
        // SMTC 只在事件发生时更新 Position；播放中要按 LastUpdatedTime 外推到「现在」。
        if playing {
            if let Ok(at) = tl.LastUpdatedTime() {
                let now = unsafe { windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime() };
                let now = ((now.dwHighDateTime as i64) << 32) | now.dwLowDateTime as i64;
                let lag = (now - at.UniversalTime) as f64 / 10_000_000.0;
                if (0.0..86_400.0).contains(&lag) {
                    position += lag;
                }
            }
        }
        if let Ok(e) = tl.EndTime() {
            duration = e.Duration as f64 / 10_000_000.0;
        }
    }
    let source=session.SourceAppUserModelId().map(|s|s.to_string()).unwrap_or_default();
    if source.eq_ignore_ascii_case("cloudmusic.exe") && duration<=0.0 {
        if let Some((current,total))=netease_timeline::timeline() {
            position=current;duration=total;sampled_at=Some(std::time::Instant::now());
        }
    }
    if source.to_ascii_lowercase().contains("kugou") && duration<=0.0 {
        if let Some(current)=netease_timeline::kugou_position() {
            position=current;sampled_at=Some(std::time::Instant::now());
        }
    }
    let album=props.AlbumTitle().map(|s|s.to_string()).unwrap_or_default();
    let key=format!("{source}|{title}|{artist}|{album}");
    let artwork=artwork.poll(key,move ||read_artwork(&props));
    let playback_type=match playback_type.map(|kind|kind.0){Some(1)=>MediaKind::Music,Some(2)=>MediaKind::Video,_=>MediaKind::Unknown};
    Some(NowPlaying { title, artist, album, source, song_id:None, playing, position, sampled_at, duration, artwork, playback_type, play_mode:PlayMode::Unknown })
}

#[cfg(not(windows))]
fn query(_artwork:&mut ArtworkCache) -> Option<NowPlaying> {
    None
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Transport {
    Toggle,
    Next,
    Prev,
    /// 跳到第几秒。
    Seek(f64),
}

/// 控制当前媒体会话（播放/暂停、上一首、下一首）。
#[cfg(windows)]
pub fn control(cmd: Transport,source:&str) {
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager;
    init_winrt();
    let session=GlobalSystemMediaTransportControlsSessionManager::RequestAsync().ok().and_then(media_result)
        .and_then(|mgr|mgr.GetSessions().ok()).and_then(|sessions|sessions.into_iter().find(|session|session.SourceAppUserModelId().ok().is_some_and(|id|id.to_string()==source)));
    let Some(session)=session else{
        if source.eq_ignore_ascii_case("cloudmusic.exe"){netease_playback::control(cmd);}
        return;
    };
    // 网易云不上报系统时间轴（EndTime 恒为 0），SMTC 的定位调用静默无效，
    // 改由原生通道点它的进度条；其余客户端仍走 SMTC。
    if let Transport::Seek(secs)=cmd {
        if source.eq_ignore_ascii_case("cloudmusic.exe") {
            let fallback=KNOWN_DURATION_MS.load(std::sync::atomic::Ordering::Relaxed) as f64/1000.0;
            if netease_playback::seek(secs,fallback) {return;}
        }
    }
    let result = match cmd {
        Transport::Toggle => session.TryTogglePlayPauseAsync(),
        Transport::Next => session.TrySkipNextAsync(),
        Transport::Prev => session.TrySkipPreviousAsync(),
        Transport::Seek(secs) => session.TryChangePlaybackPositionAsync((secs.max(0.0) * 10_000_000.0) as i64),
    };
    if let Ok(op) = result {
        let _ = media_result(op);
    }
}

#[cfg(not(windows))]
pub fn control(_cmd: Transport,_source:&str) {}

/// 该播放来源是否支持读取/切换播放模式（目前只有网易云）。
pub fn supports_play_mode(source:&str)->bool {source.eq_ignore_ascii_case("cloudmusic.exe")}

/// 切到下一个播放模式（随机 → 顺序 → 心动 → 列表循环 → 单曲循环）。目前只支持网易云。
#[cfg(windows)]
pub fn cycle_play_mode()->bool {netease_playback::cycle_play_mode()}

#[cfg(not(windows))]
pub fn cycle_play_mode()->bool {false}

/// 播放来源的可读名称：用于悬停提示与「已切换」文案。
pub fn app_label(source:&str)->String {
    let source=source.trim();
    if source.is_empty(){return "播放器".into();}
    let lower=source.to_lowercase();
    let known=[
        ("cloudmusic","网易云音乐"),
        ("qqmusic","QQ音乐"),
        ("kugou","酷狗音乐"),
        ("kuwo","酷我音乐"),
        ("sodamusic","汽水音乐"),
        ("汽水音乐","汽水音乐"),
        ("spotify","Spotify"),
        ("musicbee","MusicBee"),
        ("foobar","foobar2000"),
        ("aimp","AIMP"),
        ("potplayer","PotPlayer"),
        ("vlc","VLC"),
        ("msedge","Edge 浏览器"),
        ("chrome","Chrome"),
        ("firefox","Firefox"),
        ("brave","Brave"),
        ("opera","Opera"),
    ];
    for (key,name) in known {
        if lower.contains(key){return name.into();}
    }
    // 未知来源：去掉路径与 .exe，保留原名。
    let file=source.rsplit(['\\','/']).next().unwrap_or(source);
    if file.len()>4 && file[file.len()-4..].eq_ignore_ascii_case(".exe") {file[..file.len()-4].to_string()} else {file.to_string()}
}

/// 封面这一下是开关：客户端已经在最前面就收起来，否则切到前面。
/// 浮岛窗口带 `WS_EX_NOACTIVATE`，点它不会抢前台，所以点下的瞬间
/// `GetForegroundWindow()` 就是「客户端现在是不是在最前面」的真实答案。
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
enum WindowToggle { Show, Hide }
fn toggle_action(visible:bool,in_front:bool)->WindowToggle {
    if visible && in_front {WindowToggle::Hide}else{WindowToggle::Show}
}

/// 由浮岛藏起来的主窗口（按来源记住）：再点一次封面直接还原，
/// 不用重新枚举——窗口藏起来后打分排序可能选中别的窗口（歌词窗等）。
#[cfg(windows)]
static HIDDEN_WINDOW: Mutex<Option<(String,isize)>> = Mutex::new(None);

/// 取回上次被藏起来的窗口；来源不匹配或窗口已销毁就当作没有。
#[cfg(windows)]
fn take_hidden(source:&str)->Option<windows::Win32::Foundation::HWND> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::IsWindow;
    let mut slot=HIDDEN_WINDOW.lock().ok()?;
    let (stored,value)=slot.as_ref()?;
    if !stored.eq_ignore_ascii_case(source) {return None;}
    let window=HWND(*value as *mut core::ffi::c_void);
    *slot=None;
    unsafe{IsWindow(window).as_bool().then_some(window)}
}

#[cfg(windows)]
fn remember_hidden(source:&str,window:windows::Win32::Foundation::HWND) {
    if let Ok(mut slot)=HIDDEN_WINDOW.lock() {*slot=Some((source.to_string(),window.0 as isize));}
}

/// 判断窗口是不是「已经在最前面」：必须可见、没最小化，并且就是当前前台窗口
/// （或它的弹出子窗口）。隐藏后系统仍会把它报成前台窗口，所以可见性必须一起看。
#[cfg(windows)]
fn is_in_front(window:windows::Win32::Foundation::HWND)->bool {
    use windows::Win32::UI::WindowsAndMessaging::{GA_ROOT,GetAncestor,GetForegroundWindow,IsIconic,IsWindowVisible};
    unsafe{
        if !IsWindowVisible(window).as_bool() || IsIconic(window).as_bool() {return false;}
        let foreground=GetForegroundWindow();
        foreground==window || GetAncestor(foreground,GA_ROOT)==window
    }
}

/// 收起来：直接 `SW_HIDE`，等同于客户端自己的「关闭到托盘」，播放不中断。
#[cfg(windows)]
fn hide_window(window:windows::Win32::Foundation::HWND) {
    use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE,ShowWindow};
    unsafe{let _=ShowWindow(window,SW_HIDE);}
}

/// 把正在播放的客户端切到前台；没在运行就尝试启动。返回给用户的提示文案。
/// 只做「切窗口 / 拉起进程」这些毫秒级动作，不在这里阻塞等待。
#[cfg(windows)]
pub fn activate(source:&str)->String {
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let label=app_label(source);
    let candidates=exe_candidates(source);
    if let Some(window)=take_hidden(source) {
        focus_window(window);
        return format!("已切换到{label}");
    }
    if let Some(window)=candidates.iter().find_map(|exe|find_app_window(exe)) {
        let visible=unsafe{windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(window).as_bool()};
        match toggle_action(visible,is_in_front(window)) {
            WindowToggle::Hide=>{
                hide_window(window);
                remember_hidden(source,window);
                return format!("已隐藏{label}");
            }
            WindowToggle::Show=>{
                focus_window(window);
                return format!("已切换到{label}");
            }
        }
    }
    let launch=|target:&str| unsafe {
        let wide:Vec<u16>=target.encode_utf16().chain(std::iter::once(0)).collect();
        let op:Vec<u16>="open\0".encode_utf16().collect();
        let result=ShellExecuteW(None,PCWSTR(op.as_ptr()),PCWSTR(wide.as_ptr()),PCWSTR::null(),PCWSTR::null(),SW_SHOWNORMAL);
        result.0 as usize>32
    };
    // 1) 直接按 exe 名拉起（ShellExecute 会查 App Paths 与 PATH）。
    for exe in &candidates {
        if launch(exe){return format!("正在打开{label}");}
    }
    // 2) 开始菜单快捷方式：很多客户端只有 lnk，没有注册 App Paths。
    if let Some(lnk)=start_menu_shortcut(&label,&candidates) {
        if launch(&lnk.to_string_lossy()) {return format!("正在打开{label}");}
    }
    // 3) UWP / 已注册 AUMID 的应用走应用列表。
    if launch(&format!("shell:AppsFolder\\{source}")) {return format!("正在打开{label}");}
    format!("没找到{label}，请先手动打开一次")
}

/// 播放器来源对应的可执行文件名：按应用音量要拿它去比音频会话。
pub fn player_exe(source:&str)->String {
    #[cfg(windows)]
    {
        exe_candidates(source).into_iter().next().unwrap_or_default()
    }
    #[cfg(not(windows))]
    {
        let _=source;
        String::new()
    }
}

/// 由 SMTC 来源推导可执行文件名候选。
#[cfg(windows)]
fn exe_candidates(source:&str)->Vec<String> {
    let source=source.trim();
    if source.is_empty()||source.contains('!'){return Vec::new();}
    if source.to_lowercase().ends_with(".exe"){return vec![source.to_string()];}
    let aliases=[
        ("汽水音乐","SodaMusic.exe"),
        ("网易云音乐","cloudmusic.exe"),
        ("酷狗音乐","KuGou.exe"),
        ("酷我音乐","KwMusic.exe"),
    ];
    for (key,exe) in aliases {
        if source==key{return vec![exe.to_string()];}
    }
    vec![format!("{source}.exe")]
}

#[cfg(windows)]
unsafe fn process_file_name(pid:u32)->Option<String> {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess,QueryFullProcessImageNameW,PROCESS_NAME_WIN32,PROCESS_QUERY_LIMITED_INFORMATION};
    let process=OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION,false,pid).ok()?;
    let mut path=[0u16;1024];let mut length=path.len() as u32;
    let result=QueryFullProcessImageNameW(process,PROCESS_NAME_WIN32,PWSTR(path.as_mut_ptr()),&mut length);
    let _=CloseHandle(process);
    if result.is_err(){return None;}
    let full=String::from_utf16_lossy(&path[..length as usize]);
    Some(full.rsplit('\\').next().unwrap_or(&full).to_string())
}

/// 找该进程的顶层窗口：按「有标题栏 > 可见 > 面积大 > 有标题」打分，避免选中 IME / GDI+ 这类工具窗口。
#[cfg(windows)]
fn find_app_window(exe:&str)->Option<windows::Win32::Foundation::HWND> {
    use windows::Win32::Foundation::{BOOL,HWND,LPARAM};
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows,GetWindowLongW,GetWindowRect,GetWindowTextLengthW,GetWindowThreadProcessId,GWL_EXSTYLE,GWL_STYLE,IsWindowVisible,WS_CAPTION,WS_EX_NOACTIVATE,WS_EX_TOOLWINDOW};
    struct Search<'a>{exe:&'a str,best:Option<(u16,HWND)>}
    unsafe extern "system" fn visit(hwnd:HWND,param:LPARAM)->BOOL {
        let search=&mut *(param.0 as *mut Search);
        if search.best.is_some_and(|(score,_)|score>=12){return BOOL(0);}
        let extended=GetWindowLongW(hwnd,GWL_EXSTYLE) as u32;
        if extended & (WS_EX_TOOLWINDOW.0|WS_EX_NOACTIVATE.0)!=0{return BOOL(1);}
        let mut pid=0u32;
        GetWindowThreadProcessId(hwnd,Some(&mut pid));
        if pid==0||pid==GetCurrentProcessId(){return BOOL(1);}
        if !process_file_name(pid).is_some_and(|name|name.eq_ignore_ascii_case(search.exe)){return BOOL(1);}
        let mut score=0u16;
        if GetWindowTextLengthW(hwnd)>0{score+=2;}
        if IsWindowVisible(hwnd).as_bool(){score+=4;}
        if GetWindowLongW(hwnd,GWL_STYLE) as u32 & WS_CAPTION.0!=0{score+=4;}
        let mut rect=windows::Win32::Foundation::RECT::default();
        if GetWindowRect(hwnd,&mut rect).is_ok() {
            if rect.right-rect.left>300 && rect.bottom-rect.top>200 {score+=2;}
        }
        if search.best.is_none_or(|(old,_)|score>old){search.best=Some((score,hwnd));}
        BOOL(1)
    }
    let mut search=Search{exe,best:None};
    unsafe{let _=EnumWindows(Some(visit),LPARAM(&mut search as *mut Search as isize));}
    search.best.map(|(_,hwnd)|hwnd)
}

/// 把窗口带到前台：先常规恢复，被前台锁挡住时临时挂接输入线程再抢一次。
#[cfg(windows)]
fn focus_window(hwnd:windows::Win32::Foundation::HWND) {
    use windows::Win32::System::Threading::{AttachThreadInput,GetCurrentThreadId};
    use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    use windows::Win32::UI::WindowsAndMessaging::{BringWindowToTop,GetForegroundWindow,GetWindowThreadProcessId,IsIconic,SetForegroundWindow,ShowWindow,SW_RESTORE,SW_SHOW};
    unsafe{
        // 托盘里收起的窗口没有最小化标志，两种恢复都试一遍。
        if IsIconic(hwnd).as_bool(){let _=ShowWindow(hwnd,SW_RESTORE);}
        else{let _=ShowWindow(hwnd,SW_SHOW);}
        let _=SetForegroundWindow(hwnd);
        if GetForegroundWindow()==hwnd {return;}
        let foreground=GetForegroundWindow();
        let foreground_thread=GetWindowThreadProcessId(foreground,None);
        let current=GetCurrentThreadId();
        if foreground_thread!=0 && foreground_thread!=current {
            let _=AttachThreadInput(current,foreground_thread,true);
            let _=BringWindowToTop(hwnd);
            let _=SetForegroundWindow(hwnd);
            let _=SetFocus(hwnd);
            let _=AttachThreadInput(current,foreground_thread,false);
        }
    }
}

/// 在开始菜单里按名称找快捷方式：`exe` 精确同名优先，其次包含可读名称。
#[cfg(windows)]
fn start_menu_shortcut(label:&str,candidates:&[String])->Option<std::path::PathBuf> {
    let roots=[
        std::env::var_os("APPDATA").map(|dir|std::path::PathBuf::from(dir).join(r"Microsoft\Windows\Start Menu\Programs")),
        std::env::var_os("PROGRAMDATA").map(|dir|std::path::PathBuf::from(dir).join(r"Microsoft\Windows\Start Menu\Programs")),
    ];
    let stems:Vec<String>=candidates.iter().map(|exe|{
        let lower=exe.to_lowercase();
        if lower.ends_with(".exe"){exe[..exe.len()-4].to_string()}else{exe.clone()}
    }).collect();
    let mut stack:Vec<(std::path::PathBuf,u32)>=roots.into_iter().flatten().map(|dir|(dir,0)).collect();
    let mut loose=None;
    while let Some((dir,depth))=stack.pop() {
        if depth>3{continue;}
        let Ok(entries)=std::fs::read_dir(&dir) else{continue};
        for entry in entries.flatten() {
            let path=entry.path();
            if path.is_dir(){stack.push((path,depth+1));continue;}
            let Some(name)=path.file_stem().and_then(|name|name.to_str()) else{continue};
            if path.extension().is_none_or(|ext|!ext.eq_ignore_ascii_case("lnk")){continue;}
            if stems.iter().any(|stem|name.eq_ignore_ascii_case(stem)){return Some(path);}
            if loose.is_none() && !label.is_empty() && name.contains(label){loose=Some(path);}
        }
    }
    loose
}

#[cfg(not(windows))]
pub fn activate(_source:&str)->String {"当前系统不支持唤起播放器".into()}


#[cfg(windows)]
fn init_winrt() {
    use windows::Win32::System::Com::CoIncrementMTAUsage;
    unsafe {
        let _ = CoIncrementMTAUsage();
    }
}

#[cfg(not(windows))]
fn init_winrt() {}

// 音乐客户端可能重复上报同一播放位置，保留该位置首次采样的时间。
#[derive(Default)]
struct PlaybackClock { previous: Option<NowPlaying> }
impl PlaybackClock {
    fn update(&mut self, media: &mut NowPlaying) {
        if !media.is_music() || media.sampled_at.is_none() {
            self.previous=None;
            return;
        }
        if let Some(previous)=&self.previous {
            if media.playing && previous.playing && media.source==previous.source
                && media.title==previous.title && media.artist==previous.artist && media.album==previous.album
                && media.song_id==previous.song_id
                && media.position==previous.position && previous.sampled_at.is_some() {
                media.sampled_at=previous.sampled_at;
            }
        }
        self.previous=Some(media.clone());
    }
}

#[test]
fn kugou_repeated_samples_do_not_freeze_lyrics() {
    let mut clock=PlaybackClock::default();
    let start=std::time::Instant::now();
    let mut media=NowPlaying{source:"KuGou.exe".into(),title:"歌曲".into(),artist:"歌手".into(),
        position:42.0,playing:true,sampled_at:Some(start),..Default::default()};
    clock.update(&mut media);
    let lyrics=crate::lyrics::parse_lrc("[00:42.00]第一句\n[00:45.00]第二句");
    for second in 1..=8 {
        let now=start+Duration::from_secs(second);
        media.sampled_at=Some(now);
        clock.update(&mut media);
        let position=media.position+now.duration_since(media.sampled_at.unwrap()).as_secs_f64();
        assert_eq!(position,42.0+second as f64,"重复采样不能把进度拉回原地");
        if second>=3 {assert_eq!(lyrics.index_at(position as f32),Some(1));}
    }
    // 新位置、暂停、恢复、切歌和切换播放器必须建立新的采样基准。
    for (position,playing,title,source) in [(12.0,true,"歌曲","KuGou.exe"),(12.0,false,"歌曲","KuGou.exe"),
        (12.0,true,"歌曲","KuGou.exe"),(12.0,true,"新歌","KuGou.exe"),(12.0,true,"新歌","QQMusic.exe")] {
        let now=media.sampled_at.unwrap()+Duration::from_secs(1);
        media.position=position;media.playing=playing;media.title=title.into();media.source=source.into();media.sampled_at=Some(now);
        clock.update(&mut media);
        assert_eq!(media.sampled_at,Some(now));
        assert_eq!(media.position,position);
    }
}

#[test]
fn other_clients_repeated_samples_keep_the_clock_running() {
    for source in ["QQMusic.exe","SodaMusic.exe","Spotify.exe","cloudmusic.exe","msedge.exe"] {
        let mut clock=PlaybackClock::default();
        let start=std::time::Instant::now();
        let mut media=NowPlaying{source:source.into(),title:"歌曲".into(),artist:"歌手".into(),album:"专辑".into(),
            position:10.0,duration:120.0,playing:true,sampled_at:Some(start),..Default::default()};
        clock.update(&mut media);
        for second in 1..=5 {
            media.sampled_at=Some(start+Duration::from_secs(second));
            clock.update(&mut media);
            assert_eq!(media.sampled_at,Some(start),"{source} 重复位置不能重置采样时刻");
        }
        for position in [16.0,17.0,4.0,90.0] {
            let now=start+Duration::from_secs(10);
            media.position=position;media.sampled_at=Some(now);
            clock.update(&mut media);
            assert_eq!(media.sampled_at,Some(now),"{source} 必须跟随新位置");
        }
        for change in 0..7 {
            let now=start+Duration::from_secs(20+change);
            match change {
                0=>media.playing=false,
                1=>media.playing=true,
                2=>media.title="新歌".into(),
                3=>media.artist="另一歌手".into(),
                4=>media.album="另一专辑".into(),
                5=>media.song_id=Some(123),
                _=>media.source="另一个播放器".into(),
            }
            media.sampled_at=Some(now);clock.update(&mut media);
            assert_eq!(media.sampled_at,Some(now),"{source} 状态变化必须重置时钟");
        }
        // 会话消失或缺少有效时间轴后，恢复时不能沿用旧计时。
        clock.update(&mut NowPlaying::default());
        let now=start+Duration::from_secs(30);
        media.sampled_at=Some(now);clock.update(&mut media);assert_eq!(media.sampled_at,Some(now));
        media.sampled_at=None;clock.update(&mut media);assert_eq!(media.sampled_at,None);
        media.sampled_at=Some(now);clock.update(&mut media);assert_eq!(media.sampled_at,Some(now));
    }
}

/// 后台线程：刷新正在播放，进度条在两次刷新之间由界面按时间外推。
pub fn spawn_poller(state: Shared,ctx:eframe::egui::Context) {
    std::thread::spawn(move || {
        init_winrt();
        let mut artwork=ArtworkCache::default();
        let mut playback_clock=PlaybackClock::default();
        let mut selected_source=String::new();
        #[cfg(windows)]
        let mut netease=netease_playback::Reader::default();
        loop {
            let system=query(&mut artwork);
            #[cfg(windows)]
            let native=netease.poll();
            #[cfg(windows)]
            let native_pending=netease.pending();
            #[cfg(not(windows))]
            let native=None;
            #[cfg(not(windows))]
            let native_pending=false;
            let mut np=choose_media_for_source(system,native,&selected_source,native_pending).unwrap_or_default();
            if np.duration>0.0 {KNOWN_DURATION_MS.store((np.duration*1000.0) as u64,std::sync::atomic::Ordering::Relaxed);}
            if !np.source.is_empty(){selected_source=np.source.clone();}
            #[cfg(windows)]
            netease.fill_artwork(&mut np);
            // 播放模式目前只有网易云读得到，其他客户端保持 Unknown。
            if np.source.eq_ignore_ascii_case("cloudmusic.exe") {
                #[cfg(windows)]
                netease_playback::probe_play_mode_once();
                np.play_mode=play_mode();
            }
            playback_clock.update(&mut np);
            if np.title.is_empty() {artwork=ArtworkCache::default();}
            if let Ok(mut cur) = state.lock() {
                let changed=cur.title!=np.title || cur.artist!=np.artist || cur.playing!=np.playing
                    || cur.source!=np.source || cur.song_id!=np.song_id || cur.duration!=np.duration
                    // 播放模式也要算变化，否则切完模式界面不重绘，图标得等下次别的原因重画才更新。
                    || cur.play_mode!=np.play_mode
                    || cur.artwork.as_ref().zip(np.artwork.as_ref()).map_or(cur.artwork.is_some()!=np.artwork.is_some(),|(old,new)|!Arc::ptr_eq(old,new));
                *cur = np;
                if changed{ctx.request_repaint();}
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_click_hides_only_when_the_client_is_already_in_front(){
        assert_eq!(toggle_action(true,true),WindowToggle::Hide,"已经盖在最前面：再点一下收起");
        assert_eq!(toggle_action(true,false),WindowToggle::Show,"被别的窗口挡住：先切到前面");
        assert_eq!(toggle_action(false,false),WindowToggle::Show,"收在托盘（不可见）：先显示出来");
        assert_eq!(toggle_action(false,true),WindowToggle::Show,"不可见时即使被报成前台窗口也不能再去隐藏");
    }

    #[cfg(windows)]
    #[test]
    fn remembered_hidden_window_only_answers_its_own_source(){
        use windows::Win32::Foundation::HWND;
        remember_hidden("cloudmusic.exe",HWND(1 as *mut core::ffi::c_void));
        assert!(take_hidden("QQMusic.exe").is_none(),"别的来源不能拿走这个句柄");
        assert!(take_hidden("cloudmusic.exe").is_none(),"句柄无效时应被丢弃");
        assert!(take_hidden("cloudmusic.exe").is_none(),"取过一次就该清空，避免下次误用");
    }

    #[test]
    fn native_pause_and_loading_do_not_select_another_paused_player(){
        let native=NowPlaying{source:"cloudmusic.exe".into(),title:"网易云歌曲".into(),artist:"歌手".into(),..Default::default()};
        for source in ["QQMusic.exe","kugou","汽水音乐"] {
            let other=NowPlaying{source:source.into(),title:"其他歌曲".into(),artist:"歌手".into(),..Default::default()};
            assert_eq!(choose_media_for_source(Some(other.clone()),Some(native.clone()),"cloudmusic.exe",false).unwrap().source,"cloudmusic.exe","暂停后必须仍能控制原播放器");
            assert!(choose_media_for_source(Some(other.clone()),None,"cloudmusic.exe",true).is_none(),"加载期间不能把按钮指向其他播放器");
            assert_eq!(choose_media_for_source(Some(other.clone()),None,"cloudmusic.exe",false).unwrap().source,source,"退出后允许回到系统会话");
            let mut playing=other;playing.playing=true;
            assert_eq!(choose_media_for_source(Some(playing),Some(native.clone()),"cloudmusic.exe",false).unwrap().source,source,"其他客户端开始播放时应正常切换");
        }
    }

    #[test]
    fn slow_cover_does_not_block_metadata_or_replace_a_new_song(){
        let mut cache=ArtworkCache::default();
        let (release,wait)=std::sync::mpsc::channel();
        let start=std::time::Instant::now();
        let unblock=std::thread::spawn(move ||{std::thread::sleep(Duration::from_millis(150));release.send(()).unwrap();});
        assert!(cache.poll("old".into(),move ||{wait.recv().unwrap();Some(cover_bytes(eframe::egui::Color32::RED))}).is_none());
        assert!(start.elapsed()<Duration::from_millis(100),"封面读取不能阻塞歌曲刷新");
        assert!(cache.poll("new".into(),||None).is_none());
        unblock.join().unwrap();
        std::thread::sleep(Duration::from_millis(30));
        assert!(cache.poll("new".into(),||None).is_none(),"旧歌曲的迟到封面不能覆盖新歌曲");
    }

    #[test]
    fn native_cover_survives_a_missing_system_thumbnail(){
        let cover=Arc::new(eframe::egui::ColorImage::new([2,2],eframe::egui::Color32::BLUE));
        let native=NowPlaying{source:"cloudmusic.exe".into(),title:"歌曲".into(),artist:"歌手".into(),playing:true,artwork:Some(cover.clone()),..Default::default()};
        let mut system=native.clone();system.artwork=None;
        assert!(Arc::ptr_eq(choose_media(Some(system),Some(native)).unwrap().artwork.as_ref().expect("系统封面缺失时应保留原生封面"),&cover));
    }

    #[test]
    fn other_music_clients_keep_their_own_covers_when_netease_is_open(){
        let cover=Arc::new(eframe::egui::ColorImage::new([2,2],eframe::egui::Color32::RED));
        let native=NowPlaying{source:"cloudmusic.exe".into(),title:"网易云歌曲".into(),artist:"歌手".into(),playing:true,
            artwork:Some(Arc::new(eframe::egui::ColorImage::new([2,2],eframe::egui::Color32::BLUE))),..Default::default()};
        for source in ["SodaMusic.exe","KuGou.exe","QQMusic.exe","Spotify.exe"]{
            let system=NowPlaying{source:source.into(),title:"当前歌曲".into(),artist:"歌手".into(),playing:true,artwork:Some(cover.clone()),..Default::default()};
            let selected=choose_media(Some(system.clone()),Some(native.clone())).unwrap();
            assert_eq!(selected.source,source);assert!(Arc::ptr_eq(selected.artwork.as_ref().unwrap(),&cover));
            let mut paused=system;paused.playing=false;let mut native_paused=native.clone();native_paused.playing=false;
            assert!(Arc::ptr_eq(choose_media(Some(paused),Some(native_paused)).unwrap().artwork.as_ref().unwrap(),&cover));
            assert_eq!(choose_media(None,Some(native.clone())).unwrap().source,"cloudmusic.exe");
        }
    }

    #[test]
    fn native_music_recovers_after_the_previous_player_closes(){
        let netease=NowPlaying{source:"cloudmusic.exe".into(),title:"歌曲".into(),artist:"歌手".into(),playing:true,position:42.0,duration:240.0,..Default::default()};
        assert_eq!(choose_media(None,Some(netease.clone())).unwrap().source,"cloudmusic.exe","网易云未注册 SMTC 时仍应接回正在播放的歌曲");
        let old=NowPlaying{source:"SodaMusic.exe".into(),title:"上一首".into(),artist:"歌手".into(),..Default::default()};
        assert_eq!(choose_media(Some(old.clone()),Some(netease.clone())).unwrap().source,"cloudmusic.exe","旧客户端的暂停会话不能挡住正在播放的网易云");
        let mut current=old.clone();current.playing=true;
        assert_eq!(choose_media(Some(current),Some(netease)).unwrap().source,"SodaMusic.exe","正常播放的系统当前会话应保持优先");
        assert!(choose_media(None,None).is_none(),"播放器全部退出后不能残留歌曲");
    }

    fn cover_bytes(color:eframe::egui::Color32)->Vec<u8> {
        let image=image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(2,2,image::Rgba(color.to_array())));
        let mut bytes=std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes,image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    #[test]
    fn media_kind_checks_video_type_before_song_metadata(){
        let mut np=NowPlaying{title:"Video title".into(),artist:"Video author".into(),playback_type:MediaKind::Video,..Default::default()};
        assert_eq!(np.kind(),MediaKind::Video);
        np.playback_type=MediaKind::Music;np.title="小唐的Token的抖音 - 抖音".into();
        assert_eq!(np.kind(),MediaKind::Video,"浏览器错误上报音乐类型时仍应识别明确的视频标题");
        np.title="Movie - YouTube".into();assert_eq!(np.kind(),MediaKind::Video);
        np.title="阿拉斯加海湾".into();np.artist="蓝心羽".into();assert!(np.is_music());
        np.artist.clear();assert_eq!(np.kind(),MediaKind::Unknown,"只有页面标题不能确认是歌曲");
    }

    #[test]
    fn browser_video_authors_are_not_enough_to_identify_a_song(){
        let mut np=NowPlaying{title:"A video".into(),artist:"Video creator".into(),source:"Chrome".into(),playback_type:MediaKind::Music,..Default::default()};
        assert_eq!(np.kind(),MediaKind::Unknown);
        np.album="YouTube".into();assert_eq!(np.kind(),MediaKind::Video);
        np.title="阿拉斯加海湾".into();np.artist="蓝心羽".into();np.album="阿拉斯加海湾".into();
        assert_eq!(np.kind(),MediaKind::Music,"网易云网页版的完整歌曲元数据仍应识别为音乐");
        np.source="Spotify.exe".into();np.album.clear();assert_eq!(np.kind(),MediaKind::Music,"音乐客户端不强制要求专辑信息");
    }

    #[test]
    fn artwork_cache_retries_late_images_and_clears_on_song_change(){
        let mut cache=ArtworkCache::default();
        assert!(cache.get("first".into(),||None).is_none());
        let first=cache.get("first".into(),||Some(cover_bytes(eframe::egui::Color32::RED))).unwrap();
        let cached=cache.get("first".into(),||Some(cover_bytes(eframe::egui::Color32::RED))).unwrap();
        assert!(Arc::ptr_eq(&first,&cached));
        let missing=cache.get("first".into(),||None).unwrap();
        assert!(Arc::ptr_eq(&first,&missing),"短暂缺失时应保留当前歌曲封面");
        let invalid=cache.get("first".into(),||Some(b"invalid image".to_vec())).unwrap();
        assert!(Arc::ptr_eq(&first,&invalid),"损坏的缩略图不应清空当前有效封面");
        assert!(cache.get("second".into(),||None).is_none(),"切歌后不能保留上一首封面");
    }

    #[test]
    fn artwork_cache_replaces_a_temporary_cover_on_the_same_song(){
        let mut cache=ArtworkCache::default();
        let temporary=cache.get("song".into(),||Some(cover_bytes(eframe::egui::Color32::RED))).unwrap();
        // 网页可能先提供占位图，随后才发布真实封面。
        let actual=cache.get("song".into(),||Some(cover_bytes(eframe::egui::Color32::BLUE))).unwrap();
        assert!(!Arc::ptr_eq(&temporary,&actual),"同一首歌的真实封面到达后必须替换占位图");
        assert_eq!(actual.pixels[0],eframe::egui::Color32::BLUE);
    }

    #[test]
    fn artwork_decodes_png_jpeg_webp_and_rejects_invalid_bytes(){
        let original=image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(600,300,image::Rgb([24,120,210])));
        for format in [image::ImageFormat::Png,image::ImageFormat::Jpeg,image::ImageFormat::WebP] {
            let mut bytes=std::io::Cursor::new(Vec::new());
            original.write_to(&mut bytes,format).unwrap();
            let decoded=decode_artwork(bytes.get_ref()).unwrap();
            assert_eq!(decoded.size,[256,128]);
            assert!(decoded.pixels.iter().all(|p|p.a()==255));
        }
        assert!(decode_artwork(b"invalid image").is_none());
    }

    #[cfg(windows)]
    #[test]
    #[ignore="会连续切换当前播放器的四首歌曲，仅用于本机验收"]
    fn live_rapid_track_changes(){
        let state=shared();spawn_poller(state.clone(),eframe::egui::Context::default());
        std::thread::sleep(Duration::from_secs(2));
        let first=state.lock().unwrap().clone();
        assert!(first.is_music() && first.playing,"请先播放音乐");
        let expected=std::env::var("DAOZAI_EXPECT_SOURCE").expect("请指定实测客户端").to_lowercase();
        assert!(first.source.to_lowercase().contains(&expected),"正在播放的客户端不符：{}",first.source);
        for _ in 0..4 {
            control(Transport::Next,&first.source);
            std::thread::sleep(Duration::from_millis(400));
        }
        let start=std::time::Instant::now();
        let mut requested=String::new();
        let mut result=None;
        let mut fetched=None;
        let mut observed=String::new();
        let mut stable_since=std::time::Instant::now();
        loop {
            let media=state.lock().unwrap().clone();
            let key=format!("{}|{}|{}|{:.0}|{:?}",media.source,media.title,media.artist,media.duration,media.song_id);
            if observed!=key {observed=key.clone();stable_since=std::time::Instant::now();}
            if media.source==first.source && media.title!=first.title && requested!=key && stable_since.elapsed()>Duration::from_secs(2) {
                requested=key;
                println!("取词：{} / {} 时长={:.0}",media.title,media.artist,media.duration);
                fetched=None;
                let (tx,rx)=std::sync::mpsc::channel();result=Some(rx);
                let song=media.clone();
                std::thread::spawn(move ||{let _=tx.send(crate::lyrics::fetch_for_source(&song.source,&song.title,&song.artist,song.duration,song.song_id));});
            }
            let native=crate::lyrics::read_native(&media.source,&media.title,&media.artist,media.duration);
            if let Some(outcome)=result.as_ref().and_then(|rx|rx.try_recv().ok()) {
                match outcome {Ok(lyrics)=>fetched=Some(lyrics),Err(error)=>println!("取词结果：{error:?}")}
            }
            let lyrics=native.or_else(||fetched.clone());
            if media.source==first.source && media.title!=first.title && stable_since.elapsed()>Duration::from_secs(2) && media.artwork.is_some() && lyrics.as_ref().is_some_and(|l|l.synced) {
                println!("连续切歌后 {:.2}s：客户端={} 歌曲={} 封面已就绪 歌词={}行",start.elapsed().as_secs_f64(),media.source,media.title,lyrics.unwrap().lines.len());
                break;
            }
            assert!(start.elapsed()<Duration::from_secs(20),"切歌后未恢复：{} cover={}",media.title,media.artwork.is_some());
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    #[cfg(windows)]
    #[test]
    #[ignore="按指定来源发送一次播放控制，仅用于本机验收"]
    fn live_client_transport(){
        let source=std::env::var("DAOZAI_CONTROL_SOURCE").unwrap();
        let cmd=match std::env::var("DAOZAI_CONTROL").unwrap().as_str(){"next"=>Transport::Next,"prev"=>Transport::Prev,"toggle"=>Transport::Toggle,_=>panic!("未知控制")};
        control(cmd,&source);
    }

    #[cfg(windows)]
    #[test]
    #[ignore="需要网易云及其他播放器同时运行，验证控制不串台"]
    fn live_netease_controls_are_isolated(){
        init_winrt();
        let snapshot=||{
            let mgr=media_result(windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager::RequestAsync().unwrap()).unwrap();
            let mut rows=Vec::new();
            for session in mgr.GetSessions().unwrap() {
                let source=session.SourceAppUserModelId().unwrap().to_string();
                if source.eq_ignore_ascii_case("cloudmusic.exe"){continue;}
                let props=media_result(session.TryGetMediaPropertiesAsync().unwrap()).unwrap();
                rows.push((source,props.Title().unwrap().to_string(),session.GetPlaybackInfo().unwrap().PlaybackStatus().unwrap().0));
            }
            rows.sort();rows
        };
        let mut reader=netease_playback::Reader::default();
        for command in [Transport::Next,Transport::Prev,Transport::Toggle,Transport::Toggle] {
            std::thread::sleep(Duration::from_millis(600));
            let before=reader.poll().expect("请先打开网易云并载入歌曲");
            let others=snapshot();
            let foreground=unsafe {windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow()};
            control(command,"cloudmusic.exe");
            let start=std::time::Instant::now();
            loop {
                std::thread::sleep(Duration::from_millis(100));
                assert_eq!(snapshot(),others,"网易云控制不能改变其他客户端的歌曲或播放状态");
                assert!(start.elapsed()<Duration::from_secs(8),"网易云没有响应 {command:?}");
                let Some(after)=reader.poll() else{continue;};
                let changed=match command {Transport::Toggle=>after.playing!=before.playing,_=>after.song_id!=before.song_id};
                if changed {println!("{command:?}：{} -> {}，playing={}，其他客户端未变化",before.title,after.title,after.playing);break;}
            }
            assert_eq!(unsafe {windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow()},foreground,"后台控制不能抢占焦点");
        }
    }

    #[cfg(windows)]
    #[test]
    #[ignore="暂停指定客户端之外的系统媒体会话，隔离本机播放验收"]
    fn live_pause_other_clients(){
        init_winrt();
        let keep=std::env::var("DAOZAI_CONTROL_SOURCE").unwrap();
        let manager=media_result(windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager::RequestAsync().unwrap()).unwrap();
        for session in manager.GetSessions().unwrap() {
            let source=session.SourceAppUserModelId().unwrap().to_string();
            if source!=keep {
                if let Ok(op)=session.TryPauseAsync(){println!("暂停 {source}: {:?}",media_result(op));}
            }
        }
    }

    #[cfg(windows)]
    #[test]
    #[ignore="需要读取当前系统媒体会话"]
    fn live_sessions_metadata(){
        init_winrt();
        let manager=windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager::RequestAsync().unwrap().get().unwrap();
        for session in manager.GetSessions().unwrap() {
            let props=session.TryGetMediaPropertiesAsync().unwrap().get().unwrap();
            println!("source={} title={} artist={} type={:?} state={:?} duration={:?}",session.SourceAppUserModelId().unwrap(),props.Title().unwrap(),props.Artist().unwrap(),props.PlaybackType().ok().and_then(|kind|kind.Value().ok()),session.GetPlaybackInfo().unwrap().PlaybackStatus(),session.GetTimelineProperties().unwrap().EndTime());
        }
    }

    #[cfg(windows)]
    #[test]
    #[ignore="需要指定 DAOZAI_EXPECT_SOURCE，并在本机播放该客户端的音乐"]
    fn live_music_client_cover_lyrics_and_progress(){
        let expected=std::env::var("DAOZAI_EXPECT_SOURCE").expect("请指定实测客户端").to_lowercase();
        let state=shared();spawn_poller(state.clone(),eframe::egui::Context::default());
        let start=std::time::Instant::now();
        let first=loop{
            let media=state.lock().unwrap().clone();
            if media.source.to_lowercase().contains(&expected) && media.playing && media.artwork.is_some(){break media;}
            assert!(start.elapsed()<Duration::from_secs(15),"客户端未就绪：source={} title={} playing={} cover={}",media.source,media.title,media.playing,media.artwork.is_some());
            std::thread::sleep(Duration::from_millis(100));
        };
        let position=|media:&NowPlaying|media.position+if media.playing{media.sampled_at.map_or(0.0,|at|at.elapsed().as_secs_f64())}else{0.0};
        let before=position(&first);
        let lyrics=crate::lyrics::fetch_for_source(&first.source,&first.title,&first.artist,first.duration,first.song_id).expect("未取得歌词");
        assert!(lyrics.synced && !lyrics.lines.is_empty(),"未取得同步歌词");
        std::thread::sleep(Duration::from_secs(4));
        let next=state.lock().unwrap().clone();let after=position(&next);
        assert_eq!((&next.source,&next.title),(&first.source,&first.title),"采样期间切歌，请重新实测");
        assert!(next.playing && after>before+2.0,"播放进度没有推进：{before:.2} -> {after:.2}");
        let cover=next.artwork.expect("播放中封面消失");
        println!("客户端={} 歌曲={} 歌手={} 封面={}x{} 歌词={}行 进度={before:.2}->{after:.2} 歌词行={:?}->{:?}",
            first.source,first.title,first.artist,cover.size[0],cover.size[1],lyrics.lines.len(),lyrics.index_at(before as f32),lyrics.index_at(after as f32));
    }

    #[cfg(windows)]
    #[test]
    #[ignore="需要当前系统有提供封面的媒体会话"]
    fn live_current_session_artwork(){
        init_winrt();
        let mut cache=ArtworkCache::default();
        let start=std::time::Instant::now();
        let np=loop {
            let np=query(&mut cache).expect("当前没有媒体会话");
            if np.artwork.is_some(){break np;}
            assert!(start.elapsed()<Duration::from_secs(10),"当前媒体会话没有可读封面");
            std::thread::sleep(Duration::from_millis(100));
        };
        println!("当前媒体：{} / {}；来源 {}；类型 {:?}",np.title,np.artist,np.source,np.kind());
        let cover=np.artwork.expect("当前媒体会话没有可读封面");
        println!("系统封面尺寸：{}×{}",cover.size[0],cover.size[1]);
        if let Ok(path)=std::env::var("DAOZAI_ARTWORK_PROBE") {
            let pixels:Vec<u8>=cover.pixels.iter().flat_map(|p|p.to_srgba_unmultiplied()).collect();
            image::save_buffer(path,&pixels,cover.size[0] as u32,cover.size[1] as u32,image::ColorType::Rgba8).unwrap();
        }
    }
}
