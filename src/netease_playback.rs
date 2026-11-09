//! 网易云原生播放事件：只读增量日志，不依赖 SMTC 或特定 DLL 版本。
use super::{MediaKind,NowPlaying,PlayMode};
use std::{fs::File,io::{Read,Seek,SeekFrom},path::Path,sync::{mpsc,Arc},time::{Duration,Instant}};

const MAX_READ:u64=4*1024*1024;
const MAX_LINE:usize=256*1024;

/// 最近一次读到的时长，供「按秒定位」换算成进度条比例。
static LAST_DURATION_MS:std::sync::atomic::AtomicU64=std::sync::atomic::AtomicU64::new(0);
/// 已经点过进度条的目标位置（毫秒，-1 表示没有），轮询线程据此校正本地时间轴。
static PENDING_SEEK_MS:std::sync::atomic::AtomicI64=std::sync::atomic::AtomicI64::new(-1);
/// 上次尝试恢复播放模式的单调时刻（毫秒）。失败要能低频重试，不能一次失败就永久锁死。
static MODE_PROBE_AT:std::sync::atomic::AtomicU64=std::sync::atomic::AtomicU64::new(0);
/// UIA 兜底读取是否还在跑，避免窗口不可见时反复起线程。
static MODE_UIA_BUSY:std::sync::atomic::AtomicBool=std::sync::atomic::AtomicBool::new(false);

/// 网易云日志里的模式名（"播放模式切换" 事件）。
fn mode_from_log(name:&str)->Option<PlayMode> {
    match name {
        "playRandom"=>Some(PlayMode::Random),
        "playOrder"=>Some(PlayMode::Order),
        "playAi"|"playHeartbeat"=>Some(PlayMode::Heart),
        "playCycle"=>Some(PlayMode::Cycle),
        "playOneCycle"=>Some(PlayMode::OneCycle),
        _=>None,
    }
}

/// 日志行是 `"旧播放模式::","playRandom","新播放模式::","playOrder"`，取新值。
fn mode_from_event(args:&str)->Option<PlayMode> {
    let rest=args.split("新播放模式::").nth(1)?;
    rest.split('"')
        .map(|piece|piece.trim_matches(|c:char|c==','||c==' '))
        .find_map(mode_from_log)
}

fn monotonic_millis()->u64 {
    static CLOCK:std::sync::OnceLock<(Instant,u64)>=std::sync::OnceLock::new();
    let (start,epoch)=CLOCK.get_or_init(||(Instant::now(),std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64));
    epoch.saturating_add(start.elapsed().as_millis() as u64)
}

// 初次读取历史记录时，使用可读的本地日期恢复进度；实时推进只依赖 Instant。
fn historical_millis(line:&str,now:u64)->Option<u64> {
    use chrono::{Datelike,TimeZone};
    let date=line.strip_prefix('[')?.split(':').nth(2)?;
    let current=chrono::Local.timestamp_millis_opt(i64::try_from(now).ok()?).single()?;
    for year in [current.year(),current.year()-1] {
        if year<current.year() && !(current.month()==1 && date.starts_with("12")){break;}
        let Some(time)=chrono::NaiveDateTime::parse_from_str(&format!("{year}/{date}"),"%Y/%m%d/%H%M%S").ok()
            .and_then(|date|chrono::Local.from_local_datetime(&date).single()) else{continue;};
        let Ok(millis)=u64::try_from(time.timestamp_millis()) else{continue;};
        if millis<=now.saturating_add(1000){return Some(millis.min(now));}
    }
    None
}

// elog 的逐字节替换格式，参考 https://github.com/344272586-sketch/dsh-pet-qqmusic/blob/main/lib/netease-elog.js
fn decode(byte:u8)->u8 {
    let high=byte>>4;let low=byte&15;
    ((high^((low+8)&15))<<4)|((byte>>6)<<2)|(!high&3)
}

fn song_id(value:&serde_json::Value)->Option<u64>{value.as_u64().or_else(||value.as_str()?.parse().ok()).filter(|id|*id>0)}

fn cover_url(value:&str)->Option<String>{
    if value.len()>4096{return None;}
    let mut url=url::Url::parse(value).ok()?;
    let host=url.host_str()?;
    if !matches!(url.scheme(),"http"|"https") || !(host=="music.126.net" || host.ends_with(".music.126.net")){return None;}
    url.set_scheme("https").ok()?;url.set_query(Some("param=256y256"));url.set_fragment(None);
    Some(url.into())
}

fn track(value:&serde_json::Value)->Option<(NowPlaying,String)>{
    let t=value.get("track").unwrap_or(value);
    let id=song_id(t.get("id")?)?;
    let title=t.get("name")?.as_str()?.trim().to_owned();
    let artist=t.get("artists").and_then(|a|a.as_array()).map(|a|a.iter().filter_map(|a|a.get("name")?.as_str()).collect::<Vec<_>>().join("/")).unwrap_or_default();
    let album=t.pointer("/album/name").and_then(|v|v.as_str()).unwrap_or_default();
    if title.is_empty(){return None;}
    if [title.len(),artist.len(),album.len()].into_iter().any(|size|size>4096){crate::resources::REJECTED_DATA.fetch_add(1,std::sync::atomic::Ordering::Relaxed);return None;}
    let duration=t.get("duration").and_then(|v|v.as_f64()).unwrap_or_default()/1000.0;
    if !duration.is_finite() || !(0.0..86400.0).contains(&duration){return None;}
    let cover=t.pointer("/album/picUrl").and_then(|v|v.as_str()).and_then(cover_url).unwrap_or_default();
    Some((NowPlaying{title,artist,album:album.into(),
        source:"cloudmusic.exe".into(),song_id:Some(id),duration,playback_type:MediaKind::Music,..Default::default()},cover))
}

#[derive(Default)]
struct Playback {media:NowPlaying,artwork_url:String,at:u64,observed_at:u64,ready:bool}
impl Playback {
    fn position(&self,now:u64)->f64{
        let position=self.media.position+if self.media.playing {now.saturating_sub(self.observed_at)as f64/1000.0}else{0.0};
        // 时长明显短于实际进度时说明这条时长是错的（客户端补写的 0、上一首残留等）。
        // 用它钳制会把进度永久钉死，歌词就再也不滚动，所以只在时长还算合理时才截断。
        if self.media.duration>0.0 && position<=self.media.duration+1.0 {position.min(self.media.duration)}else{position}
    }
    fn metadata(&mut self,(metadata,cover):(NowPlaying,String)){
        self.media.title=metadata.title;self.media.artist=metadata.artist;
        self.media.album=metadata.album;
        // 网易云播放中会补写 checkPlayPrivilege 等事件，它们的 duration 字段缺失（读成 0）。
        // 用 0 覆盖已知时长，歌词就只能按歌名匹配、容易选中别的版本，进度还会被错误时长钳死，
        // 表现就是「歌词不滚」。0 一律视为未知，不覆盖。
        if metadata.duration>0.0{self.media.duration=metadata.duration;}
        self.media.source=metadata.source;self.media.playback_type=metadata.playback_type;
        if !cover.is_empty(){self.artwork_url=cover;}
    }
    fn line(&mut self,line:&str,pid:u32,now:u64){
        let Some((header,body))=line.strip_prefix('[').and_then(|s|s.split_once(']'))else{return;};
        let mut parts=header.split(':');
        if parts.next().and_then(|s|s.parse::<u32>().ok())!=Some(pid){return;}
        // 日志计数只比较事件先后，不能与本机单调时钟相减。
        let Some(at)=parts.nth(2).and_then(|s|s.parse::<u64>().ok()).filter(|at|*at>=self.at)else{return;};
        if body.contains("\"exitApp\""){*self=Self::default();return;}
        // 只解析播放事件；音频签名链接、Cookie 等其他日志内容不会进入状态或缓存。
        let Some((_,body))=body.split_once("【playing】,")else{return;};
        let Some((event,args))=body.trim().strip_prefix('"').and_then(|s|s.split_once("\","))else{return;};
        if matches!(event,"playOneTrackInPlayingList"|"checkPlayPrivilege"){
            let Some(metadata)=serde_json::from_str(args).ok().as_ref().and_then(track)else{return;};
            if self.media.song_id!=metadata.0.song_id {
                if event=="checkPlayPrivilege" && self.media.song_id.is_some(){return;}
                *self=Self{media:metadata.0,artwork_url:metadata.1,at,observed_at:now,ready:false};
            }else{self.metadata(metadata);}
        }else if event.starts_with("native") && event.contains("load完成"){
            let Some(id)=serde_json::from_str::<serde_json::Value>(args).ok().and_then(|v|song_id(v.get("songId")?))else{return;};
            if self.media.song_id!=Some(id){*self=Self::default();self.media.song_id=Some(id);}
            self.media.position=0.0;self.media.playing=true;self.at=at;self.observed_at=now;self.ready=true;
        }else if event=="setPlayingPosition" && self.media.song_id.is_some(){
            let Some(position)=args.split(',').next().and_then(|s|s.trim().parse::<f64>().ok()).filter(|p|p.is_finite() && (0.0..86400.0).contains(p))else{return;};
            self.media.position=position;self.at=at;self.observed_at=now;self.ready=true;
        }else if event.starts_with("native") && event.ends_with("state") && self.media.song_id.is_some(){
            let Some(status)=args.split(',').next().and_then(|s|s.trim().parse::<u8>().ok()).filter(|s|matches!(s,1|2))else{return;};
            self.media.position=self.position(now);self.media.playing=status==1;self.at=at;self.observed_at=now;self.ready=true;
        }
    }
    fn snapshot(&self,now:u64)->Option<NowPlaying>{
        if !self.ready || !self.media.is_music(){return None;}
        let mut media=self.media.clone();media.position=self.position(now);media.sampled_at=Some(std::time::Instant::now());
        media.event_time_ms=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()
            .map(|utc|(utc.as_millis() as u64).saturating_sub(now.saturating_sub(self.observed_at)));Some(media)
    }
}

#[derive(Default)]
struct CoverCache {
    url:String,image:Option<Arc<eframe::egui::ColorImage>>,
    pending:Option<mpsc::Receiver<Option<Arc<eframe::egui::ColorImage>>>>,
    retry_at:Option<Instant>,
    worker:Option<std::thread::JoinHandle<()>>,
}
impl CoverCache {
    fn get(&mut self,url:&str)->Option<Arc<eframe::egui::ColorImage>>{
        self.get_with(url,fetch_cover)
    }
    fn get_with(&mut self,url:&str,load:impl FnOnce(&str)->Option<Arc<eframe::egui::ColorImage>>+Send+'static)->Option<Arc<eframe::egui::ColorImage>>{
        // 切歌时丢弃旧请求的接收端，迟到的封面不能覆盖新歌。
        if self.url!=url{*self=Self{url:url.into(),worker:self.worker.take(),..Default::default()};}
        if self.url.is_empty(){return None;}
        if let Some(receiver)=&self.pending{
            match receiver.try_recv(){
                Ok(image)=>{self.image=image;self.pending=None;}
                Err(mpsc::TryRecvError::Disconnected)=>{self.pending=None;}
                Err(mpsc::TryRecvError::Empty)=>{}
            }
        }
        if self.image.is_none() && self.pending.is_none() && self.worker.as_ref().is_none_or(|worker|worker.is_finished()) && self.retry_at.is_none_or(|at|Instant::now()>=at){
            let (sender,receiver)=mpsc::channel();let url=self.url.clone();
            self.pending=Some(receiver);self.retry_at=Some(Instant::now()+Duration::from_secs(3));
            // 网络与解码不占用媒体轮询线程，歌词时间轴继续正常推进。
            self.worker=crate::resources::COVER.spawn(move||{let _=sender.send(load(&url));});
            if self.worker.is_none(){self.pending=None;}
        }
        self.image.clone()
    }
}

#[test]
fn resource_cache_recreation_cannot_bypass_download_limit(){
    use std::sync::{Condvar,Mutex,atomic::{AtomicUsize,Ordering}};
    let gate=Arc::new((Mutex::new(false),Condvar::new()));let started=Arc::new(AtomicUsize::new(0));
    for _ in 0..30 {
        let mut cache=CoverCache::default();let gate=gate.clone();let started=started.clone();
        cache.get_with("cover",move |_|{
            started.fetch_add(1,Ordering::SeqCst);
            let _released=gate.1.wait_while(gate.0.lock().unwrap(),|open|!*open).unwrap();None
        });
        std::thread::sleep(Duration::from_millis(5));
    }
    let count=started.load(Ordering::SeqCst);
    *gate.0.lock().unwrap()=true;gate.1.notify_all();
    assert!(count<=2,"重建缓存绕过了并发上限，产生 {count} 个下载线程");
}

#[test]
fn stalled_cover_requests_do_not_accumulate_threads_when_tracks_change(){
    use std::sync::{Condvar,Mutex,atomic::{AtomicUsize,Ordering}};
    let gate=Arc::new((Mutex::new(false),Condvar::new()));
    let started=Arc::new(AtomicUsize::new(0));let active=Arc::new(AtomicUsize::new(0));
    let mut cache=CoverCache::default();
    for song in 0..40 {
        let gate=gate.clone();let started=started.clone();let active=active.clone();
        cache.get_with(&format!("cover-{song}"),move |_|{
            active.fetch_add(1,Ordering::SeqCst);started.fetch_add(1,Ordering::SeqCst);
            let (lock,ready)=&*gate;let _guard=ready.wait_while(lock.lock().unwrap(),|released|!*released).unwrap();
            active.fetch_sub(1,Ordering::SeqCst);None
        });
        std::thread::sleep(Duration::from_millis(5));
    }
    let count=started.load(Ordering::SeqCst);
    *gate.0.lock().unwrap()=true;gate.1.notify_all();
    let until=Instant::now()+Duration::from_secs(2);
    while active.load(Ordering::SeqCst)>0 && Instant::now()<until {std::thread::sleep(Duration::from_millis(5));}
    assert_eq!(count,1,"旧封面请求阻塞时，切歌不能继续创建新下载线程");
    assert_eq!(active.load(Ordering::SeqCst),0,"解除阻塞后旧下载线程应结束");
    cache.worker.take().unwrap().join().unwrap();
    let image=Arc::new(eframe::egui::ColorImage::new([2,2],eframe::egui::Color32::BLUE));
    let current=image.clone();
    cache.get_with("cover-39",move |url|{assert_eq!(url,"cover-39");Some(current)});
    cache.worker.take().unwrap().join().unwrap();
    assert!(Arc::ptr_eq(&cache.get_with("cover-39",|_|panic!("已有封面不应再次下载")).unwrap(),&image),"旧请求结束后应继续加载当前歌曲");
}

#[test]
fn cover_failure_can_retry_without_a_minute_of_blank_artwork(){
    let mut cache=CoverCache::default();
    let before=Instant::now();
    cache.get("invalid-test-url");
    assert!(cache.retry_at.unwrap().duration_since(before)<Duration::from_secs(5),"封面暂时失败后不能空白一分钟");
}

fn fetch_cover(url:&str)->Option<Arc<eframe::egui::ColorImage>>{
    let url=cover_url(url)?;
    let response=ureq::AgentBuilder::new().timeout(Duration::from_secs(5)).redirects(0).build().get(&url).call().ok()?;
    let limit=2*1024*1024;let mut bytes=Vec::new();
    response.into_reader().take(limit+1).read_to_end(&mut bytes).ok()?;
    if bytes.len()>limit as usize{return None;}
    super::decode_artwork(&bytes).map(Arc::new)
}

#[derive(Default)]
pub(super) struct Reader {
    pid:u32,offset:u64,prefix:Vec<u8>,carry:Vec<u8>,skip_line:bool,
    playback:Playback,lookup:(Option<u64>,u64),cover:CoverCache,
    queue_print:Vec<Option<(std::time::SystemTime,u64)>>,
}
/// 点过进度条后把本地时间轴对齐到目标位置。必须一并重置推进起点：
/// 否则「上次播放事件到现在的整段时间」会再加到目标位置上，进度直接冲到歌曲末尾。
fn apply_pending_seek(playback:&mut Playback,now:u64){
    let pending=PENDING_SEEK_MS.swap(-1,std::sync::atomic::Ordering::Relaxed);
    if pending>=0 && playback.media.song_id.is_some() {
        playback.media.position=pending as f64/1000.0;
        playback.at=now;
        playback.observed_at=now;
    }
}

impl Reader {
    pub(super) fn pending(&self)->bool {
        self.pid!=0 && self.playback.media.song_id.is_some() && (!self.playback.ready || !self.playback.media.is_music())
    }
    /// 处理一条完整日志行。播放模式是全局状态，必须单独回读：客户端在界面上手动切档时，
    /// 这一行可能不属于当前进程的事件计数，也不带可用的播放状态，
    /// 跟着 Playback::line 的 pid / at 过滤一起丢掉就会「客户端变了、岛上不变」。
    fn ingest_line(playback:&mut Playback,pid:u32,line:&str,now:u64,replay:bool){
        if let Some(mode)=logged_mode(line){super::set_play_mode(mode);}
        let observed=if replay{historical_millis(line,now).unwrap_or(now)}else{now};
        playback.line(line,pid,observed);
    }
    fn ingest(&mut self,bytes:&[u8],now:u64,replay:bool){
        for byte in bytes.iter().copied().map(decode){
            if byte==b'\n'{
                if !self.skip_line{
                    if let Ok(line)=std::str::from_utf8(&self.carry){Self::ingest_line(&mut self.playback,self.pid,line,now,replay);}
                }
                self.carry.clear();self.skip_line=false;
            }else if !self.skip_line{
                if self.carry.len()<MAX_LINE{self.carry.push(byte);}else{self.carry.clear();self.skip_line=true;}
            }
        }
    }
    fn read(&mut self,path:&Path,pid:Option<u32>,now:u64)->Option<NowPlaying>{
        let Some(pid)=pid else{*self=Self::default();return None;};
        if self.pid!=pid{*self=Self{pid,..Self::default()};}
        let mut file=match File::open(path){Ok(file)=>file,Err(_)=>{*self=Self{pid,..Self::default()};return None;}};
        let size=file.metadata().ok()?.len();
        let mut prefix=vec![0;64.min(size as usize)];file.read_exact(&mut prefix).ok()?;
        if size<self.offset || !prefix.starts_with(&self.prefix){*self=Self{pid,..Self::default()};}
        self.prefix=prefix;
        if size>self.offset{
            let start=self.offset.max(size.saturating_sub(MAX_READ));
            if start>self.offset{self.carry.clear();self.skip_line=true;}
            file.seek(SeekFrom::Start(start)).ok()?;
            let mut bytes=vec![0;(size-start)as usize];file.read_exact(&mut bytes).ok()?;
            let replay=self.offset==0;
            self.offset=size;self.ingest(&bytes,now,replay);
        }
        // 启动时续播可能只写歌曲 ID，按该 ID 补充本地队列里的元数据。
        let id=self.playback.media.song_id;
        if id.is_some() && (!self.playback.media.is_music() || self.playback.artwork_url.is_empty() || self.playback.media.duration<=0.0) && (self.lookup.0!=id || now.saturating_sub(self.lookup.1)>=5000){
            let root=path.parent()?;
            // 队列文件可达 8MB：同一首歌且文件没有变化时不重复解析（这首歌本来就没有封面时会一直满足上面的条件）。
            let print=queue_fingerprint(root);
            if self.lookup.0!=id || self.queue_print!=print {
                self.queue_print=print;
                if let Some(metadata)=queue_track(root,id?){self.playback.metadata(metadata);}
            }
            self.lookup=(id,now);
        }
        // 时长给「按秒定位」换算比例用；点过进度条后把本地时间轴对齐到目标位置，
        // 因为这个版本的日志不再上报 setPlayingPosition，拖完不校正就会继续按旧起点外推。
        let duration=self.playback.media.duration;
        if duration>0.0 {LAST_DURATION_MS.store((duration*1000.0) as u64,std::sync::atomic::Ordering::Relaxed);}
        apply_pending_seek(&mut self.playback,now);
        self.playback.snapshot(now)
    }
    pub(super) fn poll(&mut self)->Option<NowPlaying>{
        let path=std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("NetEase/CloudMusic/cloudmusic.elog");
        let sampled=std::time::Instant::now();
        let mut media=self.read(&path,process_id(),monotonic_millis())?;
        if self.cover.url==self.playback.artwork_url{media.artwork=self.cover.image.clone();}
        media.sampled_at=Some(sampled);Some(media)
    }
    pub(super) fn fill_artwork(&mut self,media:&mut NowPlaying){
        if media.source.eq_ignore_ascii_case("cloudmusic.exe") && media.song_id==self.playback.media.song_id && media.artwork.is_none(){
            media.artwork=self.cover.get(&self.playback.artwork_url);
        }
    }
}

const QUEUE_FILES:[(&str,&str);2]=[("playingList","list"),("fmPlay","queue")];

fn queue_fingerprint(root:&Path)->Vec<Option<(std::time::SystemTime,u64)>>{
    QUEUE_FILES.iter().map(|(file,_)|std::fs::metadata(root.join("webdata/file").join(file)).ok().and_then(|meta|Some((meta.modified().ok()?,meta.len())))).collect()
}

fn queue_track(root:&Path,id:u64)->Option<(NowPlaying,String)>{
    for (file,key) in QUEUE_FILES{
        let Some(value)=(||{
            let file=File::open(root.join("webdata/file").join(file)).ok()?;
            if file.metadata().ok()?.len()>8*1024*1024{return None;}
            serde_json::from_reader::<_,serde_json::Value>(file).ok()
        })()else{continue;};
        if let Some(found)=value.get(key).and_then(|v|v.as_array()).and_then(|list|list.iter().filter_map(track).find(|t|t.0.song_id==Some(id))){return Some(found);}
    }
    None
}

fn process_id()->Option<u32>{
    use windows::{core::{w,PCWSTR},Win32::UI::WindowsAndMessaging::{FindWindowW,GetWindowThreadProcessId}};
    unsafe{let window=FindWindowW(w!("OrpheusBrowserHost"),PCWSTR::null()).ok()?;let mut pid=0;GetWindowThreadProcessId(window,Some(&mut pid));(pid!=0).then_some(pid)}
}

fn control_hotkey(command:super::Transport)->Option<&'static str>{
    match command {super::Transport::Toggle=>Some("play_pause_local"),super::Transport::Next=>Some("next_local"),
        super::Transport::Prev=>Some("prev_local"),super::Transport::Seek(_)=>None}
}

pub(super) fn control(command:super::Transport){
    use windows::{core::{w,HSTRING,PCWSTR},Win32::{Foundation::{LPARAM,WPARAM},System::DataExchange::GlobalFindAtomW,
        UI::WindowsAndMessaging::{FindWindowW,PostMessageW,WM_HOTKEY}}};
    let Some(name)=control_hotkey(command) else{return;};
    unsafe{if let Ok(window)=FindWindowW(w!("OrpheusBrowserHost"),PCWSTR::null()){
        // 读取网易云当前注册的命令，不缓存随进程变化的编号，也不广播系统媒体键。
        let atom=GlobalFindAtomW(&HSTRING::from(name));
        if atom!=0 {let _=PostMessageW(window,WM_HOTKEY,WPARAM(atom as usize),LPARAM(0));}
    }}
}

// ---------------------------------------------------------------
// 界面通道（网易云 3.x）：
// - 播放模式按钮是标准 UIA 按钮，名字随模式变化，可以直接 Invoke 切换；
// - 进度条不在 UIA 树里，也没有定位热键，只能按像素位置点它。
// ---------------------------------------------------------------
/// 底栏模式按钮的 UIA 名称 → 播放模式。
fn mode_from_button(name:&str)->Option<PlayMode> {
    match name {
        "shuffle"=>Some(PlayMode::Random),
        "order"=>Some(PlayMode::Order),
        "heartbeat"=>Some(PlayMode::Heart),
        "loop"=>Some(PlayMode::Cycle),
        "singleloop"=>Some(PlayMode::OneCycle),
        _=>None,
    }
}

#[cfg(windows)]
fn mode_button(attempts:usize)->Option<windows::Win32::UI::Accessibility::IUIAutomationElement> {
    use windows::core::{w,PCWSTR};
    use windows::Win32::System::Com::{CoCreateInstance,CoInitializeEx,CLSCTX_ALL,COINIT_MULTITHREADED};
    use windows::Win32::UI::Accessibility::{CUIAutomation,IUIAutomation,TreeScope_Descendants};
    use windows::Win32::UI::WindowsAndMessaging::FindWindowW;
    unsafe{
        let _=CoInitializeEx(None,COINIT_MULTITHREADED);
        let window=FindWindowW(w!("OrpheusBrowserHost"),PCWSTR::null()).ok()?;
        let automation:IUIAutomation=CoCreateInstance(&CUIAutomation,None,CLSCTX_ALL).ok()?;
        let root=automation.ElementFromHandle(window).ok()?;
        let condition=automation.CreateTrueCondition().ok()?;
        // 首次查询会让网易云异步构建无障碍树，前几次可能只拿到半个树，轮询等它建好。
        // 切换模式时只查一次，免得托盘态每次都白等两秒。
        for attempt in 0..attempts {
            if attempt>0 {std::thread::sleep(Duration::from_millis(400));}
            let Ok(elements)=root.FindAll(TreeScope_Descendants,&condition) else{continue;};
            for index in 0..elements.Length().unwrap_or(0) {
                let Ok(element)=elements.GetElement(index) else{continue;};
                if element.CurrentControlType().map(|kind|kind.0).unwrap_or_default()!=50000 {continue;}
                let name=element.CurrentName().map(|name|name.to_string()).unwrap_or_default();
                if mode_from_button(&name).is_some(){return Some(element);}
            }
        }
        None
    }
}

/// 读当前播放模式；首次读取会让网易云构建无障碍树，耗时约一秒。
#[cfg(windows)]
fn read_play_mode()->Option<PlayMode> {
    let button=mode_button(6)?;
    let name=unsafe{button.CurrentName().ok()?.to_string()};
    mode_from_button(&name)
}

/// 直接调用底栏模式按钮（UIA Invoke），不依赖按钮的像素位置。
#[cfg(windows)]
fn invoke_mode_button()->bool {
    use windows::Win32::UI::Accessibility::{IUIAutomationInvokePattern,UIA_InvokePatternId};
    let Some(button)=mode_button(1) else{return false;};
    unsafe{
        match button.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId) {
            Ok(pattern)=>pattern.Invoke().is_ok(),
            Err(_)=>false,
        }
    }
}

/// 从一行（已解码的）日志里读「播放模式切换」的真实新值。
fn logged_mode(line:&str)->Option<PlayMode>{
    let (_,body)=line.split_once("【playing】,")?;
    let (event,args)=body.trim().strip_prefix('"')?.split_once("\",")?;
    if event!="播放模式切换" {return None;}
    mode_from_event(args)
}

fn elog_path()->Option<std::path::PathBuf>{
    Some(std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("NetEase/CloudMusic/cloudmusic.elog"))
}

fn elog_size()->u64 {
    elog_path().and_then(|path|std::fs::metadata(path).ok()).map(|meta|meta.len()).unwrap_or(0)
}

/// 从一段（已解码的）日志文本里取最后一条模式切换事件。
/// 只有从半行开始时丢弃第一行残片，避免刚好丢掉新增日志里的第一条事件。
fn last_mode_in(text:&str)->Option<PlayMode>{
    let mut lines=text.lines();
    if !text.starts_with('['){lines.next();}
    lines.filter_map(logged_mode).last()
}

/// 扫描 `[start, 文件末尾]` 中最多 `limit` 字节，取最后一条模式切换事件。
fn scan_mode_from(path:&std::path::Path,start:u64,limit:u64)->Option<PlayMode>{
    let mut file=File::open(path).ok()?;
    let size=file.metadata().ok()?.len();
    if size<=start {return None;}
    let start=start.max(size.saturating_sub(limit));
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes=vec![0u8;(size-start) as usize];
    file.read_exact(&mut bytes).ok()?;
    let decoded:Vec<u8>=bytes.into_iter().map(decode).collect();
    last_mode_in(&String::from_utf8_lossy(&decoded))
}

/// 只读 `offset` 之后新增的日志，取最后一条模式切换事件。
/// 用作「客户端真实状态」：心动模式会跳档，岛上按固定顺序猜下一档必然越错越远。
fn logged_mode_since(offset:u64)->Option<PlayMode>{
    scan_mode_from(&elog_path()?,offset,256*1024)
}

/// 启动时恢复最近一次模式：客户端可能好几天没切过模式，256KB 尾部不足以找到事件。
/// 与 Reader 的历史重放范围保持一致。
fn last_logged_mode()->Option<PlayMode>{
    scan_mode_from(&elog_path()?,0,MAX_READ)
}

/// 底栏按钮行的位置（3.1.42 实测）：播放控制组以暂停键居中，
/// 按钮行中心距客户区底 40px，模式按钮在水平中心左侧 92px。
#[cfg(windows)]
const MODE_BUTTON_FROM_BOTTOM:i32=40;
#[cfg(windows)]
const MODE_BUTTON_FROM_CENTER:i32=92;

/// 点一下底栏的模式按钮。和进度条一样只投递窗口消息：
/// 不显示窗口、不动光标、不抢前台，网易云收在托盘时也能点。
#[cfg(windows)]
fn click_mode_button()->bool {
    use windows::Win32::Foundation::{LPARAM,RECT,WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{GetClientRect,PostMessageW,WM_LBUTTONDOWN,WM_LBUTTONUP,WM_MOUSEMOVE};
    const MK_LBUTTON:usize=0x0001;
    let Some(window)=render_widget() else{trace("mode: 找不到渲染窗口");return false;};
    unsafe{
        let mut rect=RECT::default();
        if GetClientRect(window,&mut rect).is_err(){trace("mode: GetClientRect 失败");return false;}
        let (width,height)=(rect.right-rect.left,rect.bottom-rect.top);
        if width<400 || height<300 {trace(&format!("mode: 客户区过小 {width}x{height}"));return false;}
        let x=width/2-MODE_BUTTON_FROM_CENTER;
        let y=height-MODE_BUTTON_FROM_BOTTOM;
        let point=((y as isize)<<16)|(x as isize & 0xFFFF);
        trace(&format!("mode: client={width}x{height} point=({x},{y})"));
        let _=PostMessageW(window,WM_MOUSEMOVE,WPARAM(0),LPARAM(point));
        std::thread::sleep(Duration::from_millis(40));
        let _=PostMessageW(window,WM_LBUTTONDOWN,WPARAM(MK_LBUTTON),LPARAM(point));
        std::thread::sleep(Duration::from_millis(50));
        let _=PostMessageW(window,WM_LBUTTONUP,WPARAM(0),LPARAM(point));
        std::thread::sleep(Duration::from_millis(120));
    }
    true
}

/// 切到下一个播放模式。状态一律以客户端写下的真实值为准：
/// 网易云的心动模式会跳档（单曲循环可能直接跳到心动），岛上按固定顺序猜下一档会越错越远。
#[cfg(windows)]
pub(super) fn cycle_play_mode()->bool {
    let before=elog_size();
    // 窗口消息点击实测最稳（托盘态也有效）；UIA Invoke 只在像素位置失效时兜底。
    if !click_mode_button() && !invoke_mode_button(){return false;}
    // 状态一律以客户端真实值为准：心动模式会跳档，岛上按固定顺序猜下一档会越错越远。
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(100));
        if let Some(mode)=logged_mode_since(before){
            super::set_play_mode(mode);
            return true;
        }
    }
    if let Some(mode)=read_play_mode(){super::set_play_mode(mode);return true;}
    // 点击已经投递，日志可能只是落盘慢：不能因为回读慢就弹「没能切换播放模式」。
    // 轮询线程的独立模式回读通道会在事件写入后把岛上状态同步过来。
    trace("mode: 切换命令已投递，等待轮询线程回读客户端真实模式");
    true
}

/// 播放模式恢复：主路径是 Reader 增量回读日志，这里只补「启动时尚未读到任何事件」的空隙。
/// 失败不锁死——窗口收在托盘、UIA 树还没建好时，之后每两秒还能再试，
/// 否则一次读取失败就会让模式图标永远停在 Unknown。
#[cfg(windows)]
pub(super) fn probe_play_mode_once() {
    use std::sync::atomic::Ordering;
    use windows::core::{w,PCWSTR};
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW,IsWindowVisible};
    if super::play_mode()!=PlayMode::Unknown {return;}
    let now=monotonic_millis();
    if now.saturating_sub(MODE_PROBE_AT.load(Ordering::Relaxed))<2000 {return;}
    MODE_PROBE_AT.store(now,Ordering::Relaxed);
    // 日志不依赖窗口可见性：托盘态启动也能立刻同步，不用等用户切一次模式。
    if let Some(mode)=last_logged_mode(){
        super::set_play_mode(mode);
        return;
    }
    // 日志里完全没有事件时才用 UIA 兜底；收在托盘时 UIA 树是空的，读了也是白读。
    let visible=unsafe{FindWindowW(w!("OrpheusBrowserHost"),PCWSTR::null())
        .map(|window|IsWindowVisible(window).as_bool()).unwrap_or(false)};
    if !visible || MODE_UIA_BUSY.swap(true,Ordering::Relaxed) {return;}
    std::thread::spawn(||{
        if let Some(mode)=read_play_mode(){super::set_play_mode(mode);}
        MODE_UIA_BUSY.store(false,Ordering::Relaxed);
    });
}

/// 定位过程的诊断日志：只在 debug 构建里写到临时目录，发布版本是空实现。
fn trace(message:&str){
    #[cfg(debug_assertions)]{
        use std::io::Write;
        let stamp=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d|d.as_secs()).unwrap_or(0);
        if let Ok(mut file)=std::fs::OpenOptions::new().create(true).append(true).open(std::env::temp_dir().join("daozai-seek.log")){
            let _=writeln!(file,"[{stamp}] {message}");
        }
    }
    #[cfg(not(debug_assertions))] let _=message;
}

/// 底栏高度与进度条在其中的位置按 3.1.42 实测：进度条在底栏顶部，距客户区底 82px。
#[cfg(windows)]
fn click_progress(fraction:f64)->bool {
    use windows::Win32::Foundation::{LPARAM,RECT,WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{GetClientRect,PostMessageW,WM_LBUTTONDOWN,WM_LBUTTONUP,WM_MOUSEMOVE};
    const BAR_FROM_BOTTOM:i32=82;
    const BAR_MARGIN:f64=8.0;
    const MK_LBUTTON:usize=0x0001;
    // 只把消息投给 CEF 的渲染子窗口：抬前台或移动光标都会打断用户，而顶层窗口不处理鼠标消息。
    let Some(window)=render_widget() else{trace("click: 找不到渲染窗口");return false;};
    unsafe{
        let mut rect=RECT::default();
        if GetClientRect(window,&mut rect).is_err(){trace("click: GetClientRect 失败");return false;}
        let (width,height)=(rect.right-rect.left,rect.bottom-rect.top);
        if width<400 || height<300 {trace(&format!("click: 客户区过小 {width}x{height}"));return false;}
        let x=(BAR_MARGIN+fraction.clamp(0.0,1.0)*(width as f64-BAR_MARGIN*2.0)) as i32;
        let y=height-BAR_FROM_BOTTOM;
        let point=((y as isize)<<16)|(x as isize & 0xFFFF);
        trace(&format!("click: frac={fraction:.3} client={width}x{height} point=({x},{y})"));
        let _=PostMessageW(window,WM_MOUSEMOVE,WPARAM(0),LPARAM(point));
        std::thread::sleep(Duration::from_millis(40));
        let _=PostMessageW(window,WM_LBUTTONDOWN,WPARAM(MK_LBUTTON),LPARAM(point));
        std::thread::sleep(Duration::from_millis(50));
        let _=PostMessageW(window,WM_LBUTTONUP,WPARAM(0),LPARAM(point));
    }
    true
}

/// 网易云的鼠标事件由 CEF 的渲染窗口接收：
/// OrpheusBrowserHost → CefBrowserWindow → Chrome_WidgetWin_0 → Chrome_RenderWidgetHostHWND。
#[cfg(windows)]
fn render_widget()->Option<windows::Win32::Foundation::HWND>{
    use windows::core::w;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW,GetClassNameW,GetWindow,GW_CHILD,GW_HWNDNEXT};
    unsafe fn child(parent:HWND,class:&str)->Option<HWND>{
        let mut child=GetWindow(parent,GW_CHILD).ok()?;
        loop {
            let mut name=[0u16;128];
            let length=GetClassNameW(child,&mut name) as usize;
            if String::from_utf16_lossy(&name[..length.min(128)])==class {return Some(child);}
            child=GetWindow(child,GW_HWNDNEXT).ok()?;
        }
    }
    unsafe{
        let top=FindWindowW(w!("OrpheusBrowserHost"),None).ok()?;
        let cef=child(top,"CefBrowserWindow")?;
        let host=child(cef,"Chrome_WidgetWin_0")?;
        child(host,"Chrome_RenderWidgetHostHWND")
    }
}

/// 秒 → 进度条比例。时长未知时不猜，直接放弃（宁可不跳，也不要跳错位置）。
fn seek_fraction(seconds:f64,duration:f64)->Option<f64>{
    if !(duration>0.0) || !seconds.is_finite() {return None;}
    Some((seconds.max(0.0)/duration).clamp(0.0,1.0))
}

/// 定位到第 `seconds` 秒。日志不上报位置，点击成功后由轮询线程接管本地时间轴。
/// `fallback` 是界面侧已知的总时长：日志里还没读到元数据时（续播、日志滚动）用它换算。
#[cfg(windows)]
pub(super) fn seek(seconds:f64,fallback:f64)->bool {
    use std::sync::atomic::Ordering;
    let mut duration=LAST_DURATION_MS.load(Ordering::Relaxed) as f64/1000.0;
    if !(duration>0.0) {duration=fallback;}
    let Some(fraction)=seek_fraction(seconds,duration) else{
        trace(&format!("seek: 没有时长 seconds={seconds:.2} fallback={fallback:.2}"));
        return false;
    };
    trace(&format!("seek: seconds={seconds:.2} duration={duration:.2} frac={fraction:.3}"));
    if !click_progress(fraction) {trace("seek: 点击失败");return false;}
    PENDING_SEEK_MS.store((seconds.max(0.0)*1000.0) as i64,Ordering::Relaxed);
    true
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn log_clock_origin_does_not_change_playback_position(){
        for origin in [0,9_000_000_000] {
            let mut state=Playback::default();
            state.line(&event(7,origin+1000,&metadata()),7,8_000_000);
            state.line(&event(7,origin+2000,"\"native播放资源load完成\",{\"songId\":\"123\"}"),7,8_000_000);
            assert_eq!(state.snapshot(8_001_000).unwrap().position,1.0);
            state.line(&event(7,origin+3000,"\"native播放state\",2"),7,8_002_000);
            assert_eq!(state.snapshot(8_010_000).unwrap().position,2.0);
            state.line(&event(7,origin+4000,"\"setPlayingPosition\",30"),7,8_010_000);
            state.line(&event(7,origin+5000,"\"native播放state\",1"),7,8_011_000);
            assert_eq!(state.snapshot(8_012_000).unwrap().position,31.0);
            state.line(&event(7,origin+2500,"\"setPlayingPosition\",200"),7,8_012_000);
            assert_eq!(state.snapshot(8_013_000).unwrap().position,32.0);
        }
    }
    #[test]
    fn historical_replay_restores_mid_song_and_then_uses_local_elapsed_time(){
        use chrono::TimeZone;
        let now=chrono::Local.with_ymd_and_hms(2026,11,3,17,0,13).single().unwrap().timestamp_millis() as u64;
        let raw=event(7,9_000_000_001,&metadata())
            +&event(7,9_000_000_002,"\"native播放资源load完成\",{\"songId\":\"123\"}")
            +&event(7,9_000_000_003,"\"native播放state\",2").replace("170000","170005")
            +&event(7,9_000_000_004,"\"native播放state\",1").replace("170000","170010");
        let mut reader=Reader{pid:7,..Default::default()};
        reader.ingest(&encode(&raw),now,true);
        assert_eq!(reader.playback.snapshot(now).unwrap().position,8.0);
        reader.ingest(&encode(&event(7,9_000_000_005,"\"setPlayingPosition\",40")),now+1000,false);
        assert_eq!(reader.playback.snapshot(now+3000).unwrap().position,42.0);
    }

    #[test]
    fn historical_date_handles_new_year_without_using_log_counter(){
        use chrono::TimeZone;
        let now=chrono::Local.with_ymd_and_hms(2027,1,1,0,0,2).single().unwrap().timestamp_millis() as u64;
        let line=event(7,42,"\"native播放state\",1").replace("1103/170000","1231/235959");
        assert_eq!(historical_millis(&line,now),Some(now-3000));
        let future=event(7,42,"\"native播放state\",1").replace("1103/170000","0101/000010");
        assert_eq!(historical_millis(&future,now),None,"未来时间不能被误认为去年的事件");
    }
    #[test]
    fn incomplete_artist_does_not_discard_a_known_native_song(){
        let (media,_)=track(&serde_json::json!({"id":123,"name":"待补全的歌曲","duration":120000})).unwrap();
        assert!(media.artist.is_empty() && media.is_music());assert_eq!(media.song_id,Some(123));
    }
    #[test]
    fn transport_uses_only_netease_local_commands(){
        use super::super::Transport;
        assert_eq!(control_hotkey(Transport::Toggle),Some("play_pause_local"));
        assert_eq!(control_hotkey(Transport::Next),Some("next_local"));
        assert_eq!(control_hotkey(Transport::Prev),Some("prev_local"));
        assert_eq!(control_hotkey(Transport::Seek(30.0)),None);
    }
    #[test]
    fn play_mode_names_map_to_modes(){
        use super::super::PlayMode;
        // 日志：旧 -> 新，取新值
        assert_eq!(mode_from_event("\"旧播放模式::\",\"playRandom\",\"新播放模式::\",\"playOrder\""),Some(PlayMode::Order));
        assert_eq!(mode_from_event("\"旧播放模式::\",\"playOrder\",\"新播放模式::\",\"playAi\""),Some(PlayMode::Heart));
        assert_eq!(mode_from_event("\"旧播放模式::\",\"playAi\",\"新播放模式::\",\"playCycle\""),Some(PlayMode::Cycle));
        assert_eq!(mode_from_event("\"旧播放模式::\",\"playCycle\",\"新播放模式::\",\"playOneCycle\""),Some(PlayMode::OneCycle));
        assert_eq!(mode_from_event("\"旧播放模式::\",\"playOneCycle\",\"新播放模式::\",\"playRandom\""),Some(PlayMode::Random));
        assert_eq!(mode_from_event("没有模式字段"),None);
        // UIA 按钮名（3.1.42 实测）
        for (button,mode) in [("shuffle",PlayMode::Random),("order",PlayMode::Order),("heartbeat",PlayMode::Heart),("loop",PlayMode::Cycle),("singleloop",PlayMode::OneCycle)]{
            assert_eq!(mode_from_button(button),Some(mode),"{button} 应映射到 {}",mode.label());
        }
        assert_eq!(mode_from_button("pause"),None);
    }

    #[test]
    fn logged_mode_reads_the_clients_real_new_mode(){
        use super::super::PlayMode;
        // 客户端实际写下的格式，含「旧 / 新」两个值；只认新值。
        let line=r#"[39860:46996:1109/085245:15352875:INFO:app.cpp(1231)] [2026-11-09 08:52:45] 【playing】,"播放模式切换","旧播放模式::","playAi","新播放模式::","playCycle""#;
        assert_eq!(logged_mode(line),Some(PlayMode::Cycle));
        // 心动模式会跳档（单曲循环 → 心动），岛上不能按固定顺序猜。
        let line=r#"[1:2:3] 【playing】,"播放模式切换","旧播放模式::","playOneCycle","新播放模式::","playAi""#;
        assert_eq!(logged_mode(line),Some(PlayMode::Heart));
        assert_eq!(logged_mode(r#"[1:2:3] 【playing】,"native播放state",1"#),None);
        assert_eq!(logged_mode("没有模式字段"),None);
    }

    #[test]
    fn client_mode_switch_reaches_island_even_without_playback_context(){
        use super::super::PlayMode;
        let original=super::super::play_mode();
        let mut reader=Reader::default();
        reader.pid=1234; // 与日志行的 pid 不一致：Playback::line 会把整行丢掉
        super::super::set_play_mode(PlayMode::Unknown);
        let line=r#"[39860:46996:1109/085245:15352875:INFO:app.cpp(1231)] [2026-11-09 08:52:45] 【playing】,"播放模式切换","旧播放模式::","playAi","新播放模式::","playCycle""#;
        Reader::ingest_line(&mut reader.playback,reader.pid,line,1_000_000,false);
        assert_eq!(super::super::play_mode(),PlayMode::Cycle,"客户端切档必须同步到岛上");
        assert!(reader.playback.media.song_id.is_none(),"播放状态被 pid 过滤丢弃时，模式仍要独立生效");
        super::super::set_play_mode(original);
    }

    #[test]
    fn last_mode_in_keeps_the_first_line_when_it_is_a_whole_event(){
        use super::super::PlayMode;
        let order=r#"[1:2:3] 【playing】,"播放模式切换","旧播放模式::","playRandom","新播放模式::","playOrder""#;
        let heart=r#"[1:2:3] 【playing】,"播放模式切换","旧播放模式::","playOrder","新播放模式::","playAi""#;
        // 新增日志的第一行就是模式切换时不能被当成半行残片丢掉。
        assert_eq!(last_mode_in(&format!("{order}\n")),Some(PlayMode::Order));
        assert_eq!(last_mode_in(&format!("{order}\n{heart}\n")),Some(PlayMode::Heart));
        assert_eq!(last_mode_in(&format!("半行残片\n{order}\n")),Some(PlayMode::Order));
        assert_eq!(last_mode_in("[1:2:3] 【playing】,\"native播放state\",1\n"),None);
    }

    #[test]
    fn startup_mode_recovery_scans_beyond_a_256kb_tail(){
        use super::super::PlayMode;
        // decode 是逐字节双射，反向编码出一份与真实格式一致的 elog。
        let encode=|byte:u8|(0u8..=255).find(|candidate|decode(*candidate)==byte).unwrap();
        let mode_line=format!("{}\n",r#"[1:2:3] 【playing】,"播放模式切换","旧播放模式::","playRandom","新播放模式::","playOrder""#);
        let filler="[1:2:3] 【playing】,\"native播放state\",1\n";
        let mut plain=mode_line.into_bytes();
        while plain.len()<300*1024 {plain.extend_from_slice(filler.as_bytes());}
        let encoded:Vec<u8>=plain.iter().copied().map(encode).collect();
        let unique=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let path=std::env::temp_dir().join(format!("daozai-mode-recovery-{}-{unique}.elog",std::process::id()));
        std::fs::write(&path,&encoded).unwrap();
        let narrow=scan_mode_from(&path,0,256*1024);
        let wide=scan_mode_from(&path,0,MAX_READ);
        std::fs::remove_file(&path).ok();
        assert_eq!(narrow,None,"256KB 尾部找不到久远的模式切换，这是旧实现的盲区");
        assert_eq!(wide,Some(PlayMode::Order),"启动恢复必须扫到 4MB 范围内的最近一次模式");
    }

    #[test]
    fn seek_fraction_maps_seconds_onto_the_bar(){
        assert_eq!(seek_fraction(50.0,250.0),Some(0.2));
        assert_eq!(seek_fraction(0.0,250.0),Some(0.0));
        assert_eq!(seek_fraction(-30.0,250.0),Some(0.0),"-30 秒要贴在左端，不能抛出去");
        assert_eq!(seek_fraction(999.0,250.0),Some(1.0),"> 时长要贴在右端");
        assert_eq!(seek_fraction(50.0,0.0),None,"时长未知时不跳");
        assert_eq!(seek_fraction(50.0,-1.0),None);
        assert_eq!(seek_fraction(f64::NAN,250.0),None);
    }

    #[cfg(windows)]
    #[test]
    #[ignore="实机验收：切换一次播放模式，岛上状态必须与客户端日志的真实值一致"]
    fn live_netease_mode_switch_matches_client(){
        let mut reader=Reader::default();
        let _=reader.poll();
        let before=elog_size();
        assert!(cycle_play_mode(),"模式切换命令应成功");
        let mut real=None;
        let mut island=super::super::play_mode();
        // 模拟生产轮询线程：即使日志落盘慢，独立回读通道也要把岛上状态拉齐。
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(100));
            let _=reader.poll();
            real=logged_mode_since(before).or(real);
            island=super::super::play_mode();
            if real.is_some() && island==real.unwrap(){break;}
        }
        println!("island={island:?} client_log={real:?}");
        assert!(real.is_some(),"客户端应写下「播放模式切换」事件");
        assert_eq!(island,real.unwrap(),"岛上显示的模式必须与客户端真实模式一致");
    }

    #[cfg(windows)]
    #[test]
    #[ignore="实机验收：在客户端点模式按钮后，岛上状态必须跟上"]
    fn live_client_mode_change_reaches_island(){
        let mut reader=Reader::default();
        let _=reader.poll();
        let initial=super::super::play_mode();
        let before=elog_size();
        println!("initial={initial:?} offset={offset} size={size} uia={uia:?} widget={widget} pid={pid:?}",
            offset=reader.offset,size=before,uia=read_play_mode(),widget=render_widget().is_some(),pid=process_id());
        // 等价于在客户端底栏点一下模式按钮：只往渲染窗口投递消息，不抢前台。
        assert!(click_mode_button(),"应能点到客户端的模式按钮");
        let mut real=None;
        for round in 0..20 {
            std::thread::sleep(Duration::from_millis(150));
            let _=reader.poll();
            let logged=logged_mode_since(before);
            let island=super::super::play_mode();
            if logged.is_some(){real=logged;}
            println!("round {round}: island={island:?} log={logged:?} size+{grown} offset={offset}",
                grown=elog_size().saturating_sub(before),offset=reader.offset);
            if let Some(mode)=real {
                if island==mode {println!("synced after {round} rounds: {mode:?}");return;}
            }
        }
        println!("final island={:?} log={:?}",super::super::play_mode(),real);
        panic!("客户端切换模式后岛上没有同步");
    }

    #[cfg(windows)]
    #[test]
    #[ignore="只读诊断：网易云模式的日志 / UIA / 渲染窗口通道"]
    fn live_mode_channel_probe(){
        println!("logged_mode_since(0)={:?}",logged_mode_since(0));
        println!("read_play_mode={:?}",read_play_mode());
        println!("render_widget_found={}",render_widget().is_some());
    }

    #[cfg(windows)]
    #[test]
    #[ignore="实机验收：连续多次在客户端切模式，岛上必须每轮都跟上"]
    fn live_mode_roundtrip_follows_client(){
        let mut reader=Reader::default();
        let _=reader.poll();
        for step in 0..4 {
            let before=elog_size();
            let island_before=super::super::play_mode();
            // 与用户在客户端底栏点一下等价：只往渲染窗口投递消息，不抢前台。
            assert!(click_mode_button(),"step {step}: 点不到客户端模式按钮");
            let mut real=None;
            for _ in 0..14 {
                std::thread::sleep(Duration::from_millis(120));
                let _=reader.poll();
                real=logged_mode_since(before).or(real);
                if real.is_some() && super::super::play_mode()==real.unwrap(){break;}
            }
            let island_after=super::super::play_mode();
            println!("step {step}: island_before={island_before:?} client={real:?} island_after={island_after:?}");
            assert!(real.is_some(),"step {step}: 客户端没写下模式切换日志");
            assert_eq!(island_after,real.unwrap(),"step {step}: 岛上没跟上客户端");
        }
    }

    #[test]
    fn a_metadata_event_without_duration_cannot_erase_the_known_length(){
        let mut state=Playback::default();
        state.media.song_id=Some(7);
        state.media.title="歌".into();
        state.media.duration=229.5;
        state.ready=true;
        // checkPlayPrivilege 这类事件只带歌曲 ID，duration 字段缺失会被读成 0。
        state.metadata((NowPlaying{title:"歌".into(),artist:"手".into(),duration:0.0,..Default::default()},String::new()));
        assert_eq!(state.media.duration,229.5,"0 是未知，不能覆盖已知时长");
        state.metadata((NowPlaying{title:"歌".into(),artist:"手".into(),duration:231.2,..Default::default()},String::new()));
        assert_eq!(state.media.duration,231.2,"真实时长仍然要更新");
    }

    #[test]
    fn a_wrongly_short_duration_does_not_freeze_the_position(){
        let mut state=Playback::default();
        state.media.song_id=Some(7);
        state.media.playing=true;
        state.media.duration=12.0; // 错误时长：客户端补写的旧值
        state.media.position=60.0;
        state.observed_at=1_000_000;
        state.ready=true;
        let now=1_000_000+5_000;
        assert!((state.position(now)-65.0).abs()<0.01,"进度必须继续推进，不能被错误时长钉在 12 秒");
    }

    #[test]
    fn pending_seek_resets_the_playback_clock_origin(){
        let mut state=Playback::default();
        state.media.song_id=Some(7);
        state.media.playing=true;
        state.media.duration=216.0;
        state.media.position=10.0;
        state.observed_at=1_000_000;
        state.ready=true;
        let now=1_000_000+300_000;
        PENDING_SEEK_MS.store(49_680,std::sync::atomic::Ordering::Relaxed);
        apply_pending_seek(&mut state,now);
        assert_eq!(state.position(now),49.68,"定位后应从目标秒数开始，不能叠加旧起点以来的整段时间");
        assert_eq!(state.position(now+2_000),51.68,"定位后进度要继续连续推进");
        assert_eq!(state.observed_at,now);
        assert_eq!(PENDING_SEEK_MS.load(std::sync::atomic::Ordering::Relaxed),-1,"目标只消费一次");
    }

    #[test]
    fn native_track_reads_public_album_cover_and_clears_it_on_song_change(){
        let mut state=Playback::default();
        let data=serde_json::json!({"id":123,"name":"歌曲","artists":[{"name":"歌手"}],"album":{"name":"专辑","picUrl":"http://p3.music.126.net/cover.jpg"}});
        state.line(&event(7,1000,&format!("\"playOneTrackInPlayingList\",{data}")),7,10000);
        assert_eq!(state.artwork_url,"https://p3.music.126.net/cover.jpg?param=256y256");
        state.line(&event(7,2000,"\"native播放资源load完成\",{\"songId\":\"456\"}"),7,10000);
        assert!(state.artwork_url.is_empty());
        for url in ["file:///cover.jpg","https://music.126.net.evil.test/cover.jpg","https://example.com/cover.jpg"]{assert!(cover_url(url).is_none());}
    }
    #[test]
    fn native_cover_cache_reuses_image_and_discards_late_results(){
        let image=Arc::new(eframe::egui::ColorImage::new([2,2],eframe::egui::Color32::BLUE));
        let (tx,rx)=mpsc::channel();
        let mut cache=CoverCache{url:"cover-a".into(),pending:Some(rx),..Default::default()};
        assert!(cache.get("cover-a").is_none(),"封面下载中不能阻塞媒体刷新");
        tx.send(Some(image.clone())).unwrap();
        assert!(Arc::ptr_eq(&cache.get("cover-a").unwrap(),&image));
        assert!(Arc::ptr_eq(&cache.get("cover-a").unwrap(),&image));
        let (tx,rx)=mpsc::channel();cache.pending=Some(rx);
        assert!(cache.get("").is_none());assert!(tx.send(Some(image)).is_err(),"旧请求不得把封面写入新歌");
    }
    #[test]
    fn native_cover_only_fills_the_selected_netease_song(){
        let image=Arc::new(eframe::egui::ColorImage::new([2,2],eframe::egui::Color32::BLUE));
        let mut reader=Reader::default();reader.playback.media.song_id=Some(123);reader.playback.artwork_url="cover".into();
        reader.cover=CoverCache{url:"cover".into(),image:Some(image.clone()),..Default::default()};
        for source in ["SodaMusic.exe","KuGou.exe","QQMusic.exe","Spotify.exe"]{
            let mut media=NowPlaying{source:source.into(),song_id:Some(123),..Default::default()};reader.fill_artwork(&mut media);assert!(media.artwork.is_none());
        }
        let mut media=NowPlaying{source:"cloudmusic.exe".into(),song_id:Some(456),..Default::default()};reader.fill_artwork(&mut media);assert!(media.artwork.is_none());
        media.song_id=Some(123);reader.fill_artwork(&mut media);assert!(Arc::ptr_eq(media.artwork.as_ref().unwrap(),&image));
        let system_image=Arc::new(eframe::egui::ColorImage::new([2,2],eframe::egui::Color32::RED));
        media.artwork=Some(system_image.clone());reader.fill_artwork(&mut media);assert!(Arc::ptr_eq(media.artwork.as_ref().unwrap(),&system_image));
    }
    fn event(pid:u32,at:u64,body:&str)->String{format!("[{pid}:1:1103/170000:{at}:INFO:app.cpp(1)] 【playing】,{body}\n")}
    fn metadata()->String{format!("\"playOneTrackInPlayingList\",{}",serde_json::json!({"track":{"id":"123","name":"测试歌曲","artists":[{"name":"歌手"}],"duration":240000,"album":{"name":"专辑"}}}))}
    fn encode(s:&str)->Vec<u8>{let mut table=[0;256];for b in 0..=255{table[decode(b)as usize]=b;}s.bytes().map(|b|table[b as usize]).collect()}
    #[test]
    fn native_log_tracks_load_pause_resume_seek_and_ignores_other_processes(){
        let mut state=Playback::default();
        state.line(&event(7,1000,&metadata()),7,1000);assert!(state.snapshot(1500).is_none());
        state.line(&event(7,2000,"\"native播放资源load完成，开始播放\",{\"songId\":\"123\"}"),7,2000);
        assert_eq!(state.snapshot(3000).unwrap().position,1.0);
        state.line(&event(8,3500,"\"setPlayingPosition\",99"),7,3500);
        state.line(&event(7,4000,"\"native播放state\",2"),7,4000);
        assert_eq!(state.snapshot(8000).unwrap().position,2.0);
        state.line(&event(7,5000,"\"setPlayingPosition\",30.5"),7,5000);
        state.line(&event(7,6000,"\"native播放state\",1"),7,6000);
        let media=state.snapshot(7000).unwrap();assert!(media.playing);assert_eq!(media.position,31.5);assert_eq!(media.song_id,Some(123));
        state.line(&event(7,8000,"\"setPlayingPosition\",NaN"),7,8000);assert_eq!(state.snapshot(8000).unwrap().position,32.5);
        state.line(&event(7,9000,"\"exitApp\""),7,9000);assert!(state.snapshot(10000).is_none());
    }
    #[test]
    fn native_log_handles_partial_utf8_and_new_process_without_stale_tracks(){
        let dir=std::env::temp_dir().join(format!("daozai-elog-{}",std::process::id()));std::fs::create_dir_all(&dir).unwrap();let path=dir.join("cloudmusic.elog");
        let raw=encode(&(event(7,1000,&metadata())+&event(7,2000,"\"native播放资源load完成\",{\"songId\":\"123\"}")));
        let mut reader=Reader{pid:7,..Default::default()};
        for byte in &raw{reader.ingest(&[*byte],3000,false);}
        assert_eq!(reader.playback.snapshot(3000).unwrap().title,"测试歌曲");
        std::fs::write(&path,&raw).unwrap();reader=Reader::default();
        assert_eq!(reader.read(&path,Some(7),3000).unwrap().position,0.0);
        assert_eq!(reader.read(&path,Some(7),4000).unwrap().position,1.0);
        assert!(reader.read(&path,None,4000).is_none());
        assert!(reader.read(&path,Some(8),4000).is_none(),"新进程不能继承旧进程的歌曲");
        let raw=encode(&(event(8,5000,&metadata())+&event(8,6000,"\"native播放资源load完成\",{\"songId\":\"123\"}")));
        std::fs::write(&path,&raw).unwrap();assert_eq!(reader.read(&path,Some(8),7000).unwrap().position,0.0);
        std::fs::write(&path,b"").unwrap();assert!(reader.read(&path,Some(8),7000).is_none());
        std::fs::remove_file(path).unwrap();std::fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn native_log_resumes_from_queue_metadata_without_any_smtc_session(){
        let dir=std::env::temp_dir().join(format!("daozai-elog-queue-{}",std::process::id()));let queue=dir.join("webdata/file");std::fs::create_dir_all(&queue).unwrap();
        let data=serde_json::json!({"list":[{"track":{"id":123,"name":"续播歌曲","artists":[{"name":"歌手"}],"duration":240000,"album":{"picUrl":"https://p1.music.126.net/resume.jpg"}}}]});
        std::fs::write(queue.join("playingList"),data.to_string()).unwrap();let path=dir.join("cloudmusic.elog");
        std::fs::write(&path,encode(&(event(9,1000,"\"native播放资源load完成\",{\"songId\":\"123\"}")+&event(9,1100,"\"setPlayingPosition\",80")))).unwrap();
        let mut reader=Reader::default();let media=reader.read(&path,Some(9),2100).unwrap();assert_eq!(media.title,"续播歌曲");assert_eq!(media.position,80.0);
        assert_eq!(reader.playback.artwork_url,"https://p1.music.126.net/resume.jpg?param=256y256");
        std::fs::remove_file(path).unwrap();std::fs::remove_file(queue.join("playingList")).unwrap();std::fs::remove_dir(&queue).unwrap();std::fs::remove_dir(dir.join("webdata")).unwrap();std::fs::remove_dir(dir).unwrap();
    }
    #[test]
    #[ignore="需要本机网易云正在播放，用于验证无 SMTC 的原生数据和歌词"]
    fn live_native_netease_playback_and_lyrics(){
        let mut reader=Reader::default();let first=reader.poll().expect("未读到网易云原生播放事件");
        println!("歌曲={} 歌手={} ID={:?} 播放={} 进度={:.2}/{:.2}",first.title,first.artist,first.song_id,first.playing,first.position,first.duration);
        let lyrics=crate::lyrics::fetch_for_source(&first.source,&first.title,&first.artist,first.duration,first.song_id).unwrap();
        assert!(lyrics.synced && !lyrics.lines.is_empty());println!("已获取 {} 行同步歌词",lyrics.lines.len());
        std::thread::sleep(std::time::Duration::from_secs(2));let next=reader.poll().unwrap();
        if first.playing && next.playing && first.song_id==next.song_id{assert!(next.position>first.position);}
    }
    #[test]
    #[ignore="需要本机网易云正在播放，用于验证原生封面与缓存"]
    fn live_native_netease_album_cover(){
        let mut reader=Reader::default();let start=Instant::now();
        loop{
            let mut media=reader.poll().expect("未读到网易云原生播放事件");reader.fill_artwork(&mut media);
            if let Some(image)=&media.artwork{
                assert!(image.size[0]>1 && image.size[0]<=256 && image.size[1]>1 && image.size[1]<=256);
                let mut next=reader.poll().unwrap();reader.fill_artwork(&mut next);
                if next.song_id==media.song_id{assert!(Arc::ptr_eq(next.artwork.as_ref().unwrap(),image));}
                println!("歌曲={} 封面={}x{}，重复采样复用缓存",media.title,image.size[0],image.size[1]);break;
            }
            assert!(start.elapsed()<Duration::from_secs(10),"未获取到网易云专辑封面");std::thread::sleep(Duration::from_millis(100));
        }
    }
}
