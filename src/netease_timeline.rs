//! 网易云、酷狗未上报系统时间轴时，只读播放组件里的播放时间。
//! 已验证的版本记在适配表里；不在表里的版本，播放时按「随真实时间同步增长」的特征自动找到进度并记住。
use super::music_compat::{self as compat,Item,Support};
use std::{ffi::c_void,time::{Duration,Instant}};
type Handle = *mut c_void;
#[link(name="kernel32")]
extern "system" {
    fn OpenProcess(access:u32,inherit:i32,pid:u32)->Handle;
    fn CloseHandle(handle:Handle)->i32;
    fn ReadProcessMemory(handle:Handle,address:usize,buffer:*mut c_void,size:usize,read:*mut usize)->i32;
    fn K32EnumProcessModules(handle:Handle,modules:*mut usize,size:u32,needed:*mut u32)->i32;
    fn K32GetModuleBaseNameW(handle:Handle,module:usize,name:*mut u16,size:u32)->u32;
    fn K32GetModuleFileNameExW(handle:Handle,module:usize,name:*mut u16,size:u32)->u32;
}
struct Process(Handle);
impl Drop for Process {fn drop(&mut self){unsafe{CloseHandle(self.0);}}}
impl Process {
    /// 先用有限查询权限核对进程身份，再申请读取权限（进程号可能被其他软件复用）。
    fn open(pid:u32,exe:&str)->Option<Self> {
        if !super::player_process::image_is(pid,exe){return None;}
        let p=Process(unsafe{OpenProcess(0x410,0,pid)});
        (!p.0.is_null() && name(p.0,0).eq_ignore_ascii_case(exe)).then_some(p)
    }
}
fn read<const N:usize>(p:Handle,address:usize)->Option<[u8;N]> {
    let mut bytes=[0;N];let mut count=0;
    (unsafe{ReadProcessMemory(p,address,bytes.as_mut_ptr().cast(),N,&mut count)}!=0 && count==N).then_some(bytes)
}
fn read_f64(p:Handle,address:usize)->Option<f64> {read::<8>(p,address).map(f64::from_le_bytes)}
fn read_into(p:Handle,address:usize,buffer:&mut [u8])->bool {
    let mut count=0;
    (unsafe{ReadProcessMemory(p,address,buffer.as_mut_ptr().cast(),buffer.len(),&mut count)}!=0) && count==buffer.len()
}
fn valid(position:f64,duration:f64)->bool {
    position.is_finite() && duration.is_finite() && duration>0.0 && duration<86400.0 && position>=0.0 && position<=duration
}
#[test]
fn rejects_invalid_timeline(){
    assert!(!valid(f64::NAN,240.0));assert!(!valid(0.0,0.0));assert!(!valid(241.0,240.0));assert!(!valid(-1.0,240.0));assert!(valid(12.01,275.5));
}
fn name(p:Handle,module:usize)->String {
    let mut s=[0u16;260];let n=unsafe{K32GetModuleBaseNameW(p,module,s.as_mut_ptr(),260)} as usize;
    String::from_utf16_lossy(&s[..n])
}

/// 网易云进度与时长；自动适配的版本只有进度，时长为 0（由网易云本机数据补齐）。
pub fn timeline(playing:bool)->Option<(f64,f64)> {query(false,playing)}
pub fn kugou_position(playing:bool)->Option<f64> {query(true,playing).map(|(position,_)|position)}

fn valid_position(position:f64)->bool {position.is_finite() && (0.0..86400.0).contains(&position)}
#[test]
fn kugou_rejects_invalid_positions(){
    assert!(valid_position(0.0));assert!(valid_position(214.358));
    for invalid in [f64::NAN,f64::INFINITY,-1.0,86400.0]{assert!(!valid_position(invalid));}
}

// ---- 适配表 ----

/// 一个已验证的播放组件版本：PE 时间戳、映像大小，以及进度（秒，f64）和时长在组件内的位置。
struct Known {stamp:u32,size:u32,position:usize,duration:Option<usize>}
struct Player {exe:&'static str,module:&'static str,window:&'static str,topic:&'static str,known:&'static [Known]}

/// 新验证一个版本，在对应播放器的 `known` 里加一行即可。
const KUGOU:Player=Player{exe:"KuGou.exe",module:"kgplayer.dll",window:"kugou_ui",topic:"酷狗进度",known:&[
    Known{stamp:0x6ac9ec86,size:4587520,position:0x3949f8,duration:None},
]};
const NETEASE:Player=Player{exe:"cloudmusic.exe",module:"cloudmusic.dll",window:"OrpheusBrowserHost",topic:"网易云进度",known:&[
    Known{stamp:0x6ad712e1,size:41054208,position:0x21a74d8,duration:Some(0x2214778)},
]};
fn player(kugou:bool)->&'static Player {if kugou{&KUGOU}else{&NETEASE}}

/// 组件的编译时间早于适配表里所有已验证版本：说明是旧版本，更新播放器即可。
fn older_than_table(player:&Player,stamp:u32)->bool {player.known.iter().all(|known|stamp<known.stamp)}

/// 读取方式：来自适配表，或自动适配学到的进度位置。
#[derive(Clone,Copy,PartialEq,Debug)]
struct Layout {position:usize,duration:Option<usize>,learned:bool}

fn layout_for(player:&Player,stamp:u32,size:u32)->Option<Layout> {
    if let Some(known)=player.known.iter().find(|k|k.stamp==stamp && k.size==size) {
        return Some(Layout{position:known.position,duration:known.duration,learned:false});
    }
    compat::learned(player.module,stamp,size).map(|position|Layout{position,duration:None,learned:true})
}

// ---- 查找与读取 ----

/// 已找到的进程与模块基址；每轮只需读几个字节，不再枚举全部进程和模块。
#[derive(Clone,Copy)]
struct Found {pid:u32,base:usize,stamp:u32,size:u32,layout:Layout}

#[derive(Default)]
struct Slot {
    found:Option<Found>,missed:Option<Instant>,
    /// 无法适配的进程号：同一进程不再反复做进程快照（版本在进程存活期间不会变）。
    rejected:Option<u32>,
    learner:Option<Learner>,
    /// 自动适配的位置是否还有效：播放中读数长时间不变就作废重学。
    stuck:Option<(f64,Instant)>,
}
static SLOTS:std::sync::Mutex<[Option<Slot>;2]>=std::sync::Mutex::new([None,None]);
/// 没找到（未运行或无法适配）时，重新枚举的最短间隔。
const REDISCOVER:Duration=Duration::from_secs(3);
/// 自动适配的位置在播放中这么久读数不变，视为失效。
const STUCK:Duration=Duration::from_secs(8);

fn query(kugou:bool,playing:bool)->Option<(f64,f64)> {
    let player=player(kugou);
    let mut slots=SLOTS.lock().unwrap_or_else(|e|e.into_inner());
    let slot=slots[kugou as usize].get_or_insert_with(Slot::default);
    if let Some(found)=slot.found {
        match sample(player,found) {
            Some(value)=>{
                if found.layout.learned && invalid_learned(slot,value.map(|v|v.0),playing) {
                    compat::forget(player.module,found.stamp,found.size);
                    super::music_log::state(player.topic,"自动适配的位置已失效，重新适配");
                    slot.found=None;slot.stuck=None;
                    slot.learner=Some(Learner::new(found.pid,found.base,found.stamp,found.size,0));
                    return None;
                }
                return value;
            }
            None=>{slot.found=None;slot.stuck=None;compat::clear(player.exe,Item::Progress);}
        }
    }
    if let Some(mut learner)=slot.learner.take() {
        let Some(p)=Process::open(learner.pid,player.exe) else{compat::clear(player.exe,Item::Progress);return None;};
        match learner.step(&p,playing,Instant::now()) {
            Learn::Pending=>{slot.learner=Some(learner);return None;}
            Learn::Found(position)=>{
                let (pid,base,stamp,size)=(learner.pid,learner.base,learner.stamp,learner.size);
                compat::remember(player.module,stamp,size,position);
                super::music_log::state(player.topic,&format!("已自动适配：{} 时间戳{stamp:08x} 大小{size} 进度位置{position:#x}",player.module));
                compat::set(player.exe,Item::Progress,Support::Learned);
                let found=Found{pid,base,stamp,size,layout:Layout{position,duration:None,learned:true}};
                slot.found=Some(found);
                return sample(player,found).flatten();
            }
            Learn::Failed(reason)=>{
                let attempts=learner.attempts+1;
                if attempts<LEARN_ATTEMPTS {
                    slot.learner=Some(Learner::new(learner.pid,learner.base,learner.stamp,learner.size,attempts));
                    return None;
                }
                slot.rejected=Some(learner.pid);slot.missed=Some(Instant::now());
                let older=older_than_table(player,learner.stamp);
                let reason=format!("{} 版本不在适配表里，自动适配失败（{reason}）{}",player.module,if older{"，比已适配的版本旧，提醒用户更新"}else{"，比已适配的版本新，待补适配"});
                super::music_log::state(player.topic,&reason);
                compat::set(player.exe,Item::Progress,Support::Unavailable(reason));
                if older {compat::outdated(player.exe);}
                return None;
            }
        }
    }
    if slot.missed.is_some_and(|at|at.elapsed()<REDISCOVER){return None;}
    if let Some(pid)=slot.rejected {
        // 播放器窗口仍属于那个无法适配的进程：只看一次窗口（微秒级），不做十几毫秒的进程快照。
        if window_pid(player)==Some(pid) && super::player_process::image_is(pid,player.exe) {slot.missed=Some(Instant::now());return None;}
        slot.rejected=None;
    }
    match discover(player) {
        Ok(found)=>{
            slot.found=Some(found);slot.missed=None;slot.stuck=None;
            let (state,support)=if found.layout.learned {("已自动适配（沿用上次学到的位置），读取内存进度",Support::Learned)}else{("已适配，读取内存进度",Support::Ok)};
            super::music_log::state(player.topic,state);
            compat::set(player.exe,Item::Progress,support);
            sample(player,found).flatten()
        }
        Err(Miss{reason,unknown:Some((pid,base,stamp,size))})=>{
            // 不在适配表里：开始按数据特征自动适配，需要播放几秒。
            super::music_log::state(player.topic,&format!("{reason}，开始自动适配"));
            compat::set(player.exe,Item::Progress,Support::Learning);
            slot.learner=Some(Learner::new(pid,base,stamp,size,0));
            None
        }
        Err(Miss{reason,unknown:None})=>{
            slot.missed=Some(Instant::now());
            super::music_log::state(player.topic,&reason);
            compat::clear(player.exe,Item::Progress);
            None
        }
    }
}

/// 自动适配的位置：播放中读数无效，或持续 `STUCK` 不变，即作废。
fn invalid_learned(slot:&mut Slot,position:Option<f64>,playing:bool)->bool {
    let Some(position)=position else{return playing;};
    if !playing {slot.stuck=None;return false;}
    match slot.stuck {
        Some((value,since)) if (value-position).abs()<0.01 => since.elapsed()>=STUCK,
        _=>{slot.stuck=Some((position,Instant::now()));false}
    }
}

/// 外层 None 表示进程或模块已失效，需要重新查找；内层 None 只是本轮读数无效。
fn sample(player:&Player,found:Found)->Option<Option<(f64,f64)>> {
    let p=Process::open(found.pid,player.exe)?;
    // 每轮都核对 PE 标识（只读几个字节），模块被卸载或换版本时重新查找。
    let (stamp,size)=pe_identity(p.0,found.base)?;
    if stamp!=found.stamp || size!=found.size {return None;}
    let position=read_f64(p.0,found.base+found.layout.position)?;
    let Some(offset)=found.layout.duration else{return Some(valid_position(position).then_some((position,0.0)));};
    let duration=read_f64(p.0,found.base+offset)?;
    Some(valid(position,duration).then_some((position,duration)))
}

fn pe_identity(p:Handle,base:usize)->Option<(u32,u32)> {
    let pe=u32::from_le_bytes(read::<4>(p,base+60)?) as usize;
    if pe>4096{return None;}
    Some((u32::from_le_bytes(read::<4>(p,base+pe+8)?),u32::from_le_bytes(read::<4>(p,base+pe+80)?)))
}

/// 没找到的原因；`unknown` 是组件版本不在适配表、也没有学到过的进程与模块（之后自动适配）。
struct Miss {reason:String,unknown:Option<(u32,usize,u32,u32)>}
impl From<String> for Miss {fn from(reason:String)->Self{Self{reason,unknown:None}}}

/// 播放器主窗口所属的进程号：查一次窗口，不需要进程快照。
fn window_pid(player:&Player)->Option<u32>{
    use windows::{core::{HSTRING,PCWSTR},Win32::UI::WindowsAndMessaging::{FindWindowW,GetWindowThreadProcessId}};
    let class=HSTRING::from(player.window);
    let window=unsafe{FindWindowW(&class,PCWSTR::null())}.ok()?;
    let mut pid=0;unsafe{GetWindowThreadProcessId(window,Some(&mut pid));}
    (pid!=0).then_some(pid)
}

fn discover(player:&Player)->Result<Found,Miss> {
    discover_in(player.exe,player.module,|stamp,size|layout_for(player,stamp,size))
}

fn discover_in(process:&str,module:&str,layout:impl Fn(u32,u32)->Option<Layout>)->Result<Found,Miss> {
    let mut reason=format!("未找到 {process}");
    // 先从进程快照按名称筛出播放器，只向核实后的播放器申请内存读取权限。
    for id in super::player_process::pids_named(process) {
        if !super::player_process::image_is(id,process){continue;}
        let p=Process(unsafe{OpenProcess(0x410,0,id)});
        if p.0.is_null(){reason=format!("无法读取 {process}（权限不足）");continue;}
        if !name(p.0,0).eq_ignore_ascii_case(process){continue;}
        let mut modules=[0usize;1024];let mut size=0;
        if unsafe{K32EnumProcessModules(p.0,modules.as_mut_ptr(),std::mem::size_of_val(&modules) as u32,&mut size)}==0{continue;}
        reason=format!("{process} 未加载 {module}");
        for &base in modules.iter().take(size as usize/std::mem::size_of::<usize>()) {
            if !name(p.0,base).eq_ignore_ascii_case(module){continue;}
            let Some((stamp,image_size))=pe_identity(p.0,base) else{return Err(format!("无法读取 {module} 文件头").into());};
            if let Some(layout)=layout(stamp,image_size) {return Ok(Found{pid:id,base,stamp,size:image_size,layout});}
            let version=module_path(p.0,base).map_or_else(||"未知".into(),|path|super::music_log::file_version(&path));
            return Err(Miss{reason:format!("{module} 版本不在适配表里：{version} 时间戳{stamp:08x} 大小{image_size}"),unknown:Some((id,base,stamp,image_size))});
        }
    }
    Err(reason.into())
}

fn module_path(p:Handle,module:usize)->Option<String> {
    let mut s=[0u16;1024];let n=unsafe{K32GetModuleFileNameExW(p,module,s.as_mut_ptr(),1024)} as usize;
    (n>0).then(||String::from_utf16_lossy(&s[..n]))
}

// ---- 自动适配：在组件的可写数据区里找「播放时随真实时间同步增长」的秒数 ----

/// 每个进程最多尝试这么多轮（换歌、拖动会让一轮失败）。
const LEARN_ATTEMPTS:u8=3;
/// 两次采样的最短间隔；太近时播放器可能还没更新读数。
const LEARN_GAP:Duration=Duration::from_millis(700);
/// 两次采样相隔太久（轮询变慢、暂停）就重新开始这一步。
const LEARN_MAX_GAP:Duration=Duration::from_secs(5);
/// 可写数据区总量上限，超过则放弃（防异常组件）。
const LEARN_LIMIT:usize=64*1024*1024;
/// 确认需要连续通过的轮数（约 2～3 秒播放）。
const LEARN_ROUNDS:u8=3;

enum Learn {Pending,Found(usize),Failed(&'static str)}

struct Learner {
    pid:u32,base:usize,stamp:u32,size:u32,attempts:u8,
    sections:Option<Vec<(usize,usize)>>,
    /// 第一次整段读取（各可写区原始字节）。
    full:Option<Vec<Vec<u8>>>,
    /// 通过检查的候选：组件内偏移与上次读数。
    candidates:Vec<(usize,f64)>,
    last:Option<Instant>,rounds:u8,
}

impl Learner {
    fn new(pid:u32,base:usize,stamp:u32,size:u32,attempts:u8)->Self {
        Self{pid,base,stamp,size,attempts,sections:None,full:None,candidates:Vec::new(),last:None,rounds:0}
    }
    fn reset(&mut self){self.full=None;self.candidates.clear();self.last=None;self.rounds=0;}

    fn step(&mut self,p:&Process,playing:bool,now:Instant)->Learn {
        if !playing {self.reset();return Learn::Pending;}
        if self.last.is_some_and(|at|now.duration_since(at)<LEARN_GAP){return Learn::Pending;}
        if self.last.is_some_and(|at|now.duration_since(at)>LEARN_MAX_GAP){self.reset();}
        if self.sections.is_none() {
            match writable_sections(p.0,self.base) {
                Some(sections) if sections.iter().map(|s|s.1).sum::<usize>()<=LEARN_LIMIT=>self.sections=Some(sections),
                Some(_)=>return Learn::Failed("组件数据区过大"),
                None=>return Learn::Failed("无法读取组件结构"),
            }
        }
        let sections=self.sections.clone().unwrap_or_default();
        let gap=self.last.map(|at|now.duration_since(at).as_secs_f64());
        self.last=Some(now);
        if self.full.is_none() && self.candidates.is_empty() {
            let mut data=Vec::new();
            for &(rva,len) in &sections {
                let mut bytes=vec![0u8;len];
                if !read_into(p.0,self.base+rva,&mut bytes){bytes.clear();}
                data.push(bytes);
            }
            self.full=Some(data);
            return Learn::Pending;
        }
        let Some(gap)=gap else{return Learn::Pending;};
        if let Some(full)=self.full.take() {
            for (&(rva,len),old) in sections.iter().zip(&full) {
                if old.len()!=len {continue;}
                let mut new=vec![0u8;len];
                if !read_into(p.0,self.base+rva,&mut new){continue;}
                self.candidates.extend(advancing(old,&new,gap).into_iter().map(|(offset,value)|(rva+offset,value)));
                if self.candidates.len()>4096 {return Learn::Failed("候选过多");}
            }
        } else {
            let base=self.base;
            self.candidates.retain_mut(|(offset,value)|{
                let Some(now_value)=read_f64(p.0,base+*offset) else{return false;};
                let ok=advanced(*value,now_value,gap);*value=now_value;ok
            });
        }
        self.rounds+=1;
        decide(&self.candidates,self.rounds)
    }
}

/// 两次读数之间是否像播放进度：都在 0～1 小时内，增长量约等于间隔时间。
fn advanced(old:f64,new:f64,gap:f64)->bool {
    let tolerance=0.35+gap*0.1;
    old.is_finite() && new.is_finite() && (0.0..3600.0).contains(&old) && (0.0..3600.0).contains(&new) && (new-old-gap).abs()<=tolerance
}

/// 逐个 8 字节对齐的 f64 比较两份数据，返回像播放进度的偏移与新读数。
fn advancing(old:&[u8],new:&[u8],gap:f64)->Vec<(usize,f64)> {
    old.chunks_exact(8).zip(new.chunks_exact(8)).enumerate().filter_map(|(i,(a,b))|{
        let (a,b)=(f64::from_le_bytes(a.try_into().ok()?),f64::from_le_bytes(b.try_into().ok()?));
        advanced(a,b,gap).then_some((i*8,b))
    }).collect()
}

/// 连续通过 `LEARN_ROUNDS` 轮后定论：取读数最小的候选（同一读数的多个副本取偏移最小的）。
/// 同步增长的其他计时只会比播放位置大，例如酷狗紧挨着进度的解码位置总是领先几秒；
/// 候选太多说明特征不够明确，放弃。
fn decide(candidates:&[(usize,f64)],rounds:u8)->Learn {
    if candidates.is_empty() {return Learn::Failed("没有找到随播放增长的读数");}
    if rounds<LEARN_ROUNDS {return Learn::Pending;}
    if candidates.len()>8 {return Learn::Failed("随播放增长的读数太多，无法确定");}
    let low=candidates.iter().map(|(_,v)|*v).fold(f64::MAX,f64::min);
    Learn::Found(candidates.iter().filter(|(_,v)|*v-low<=0.5).map(|(offset,_)|*offset).min().unwrap_or_default())
}

/// 组件里可写（数据）节的相对位置和大小。
fn writable_sections(p:Handle,base:usize)->Option<Vec<(usize,usize)>> {
    let pe=u32::from_le_bytes(read::<4>(p,base+60)?) as usize;
    if pe>4096 {return None;}
    let count=u16::from_le_bytes(read::<2>(p,base+pe+6)?) as usize;
    let optional=u16::from_le_bytes(read::<2>(p,base+pe+20)?) as usize;
    if count>96 {return None;}
    let table=base+pe+24+optional;
    let mut sections=Vec::new();
    for i in 0..count {
        let header=read::<40>(p,table+i*40)?;
        let size=u32::from_le_bytes(header[8..12].try_into().ok()?) as usize;
        let rva=u32::from_le_bytes(header[12..16].try_into().ok()?) as usize;
        let flags=u32::from_le_bytes(header[36..40].try_into().ok()?);
        if flags&0x8000_0000!=0 && size>0 {sections.push((rva,(size+7)&!7));}
    }
    Some(sections)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_reports_why_a_player_is_not_adapted(){
        let me=std::env::current_exe().unwrap();
        let exe=me.file_name().unwrap().to_str().unwrap().to_owned();
        let none=discover_in("不存在的播放器.exe","x.dll",|_,_|None).err().unwrap();
        assert_eq!((none.reason.as_str(),none.unknown.is_none()),("未找到 不存在的播放器.exe",true));
        let missing=discover_in(&exe,"不存在的组件.dll",|_,_|None).err().unwrap();
        assert_eq!((missing.reason,missing.unknown.is_none()),(format!("{exe} 未加载 不存在的组件.dll"),true),"组件可能稍后才加载，之后要照常重试");
        let unknown=discover_in(&exe,"kernel32.dll",|_,_|None).err().unwrap();
        assert!(unknown.reason.starts_with("kernel32.dll 版本不在适配表里：") && !unknown.reason.contains("未知"),"{}",unknown.reason);
        let (pid,..)=unknown.unknown.expect("不在表里的版本交给自动适配");
        assert!(super::super::player_process::image_is(pid,&exe));
        let layout=Layout{position:8,duration:None,learned:true};
        assert!(discover_in(&exe,"kernel32.dll",|_,_|Some(layout)).is_ok_and(|found|found.layout==layout),"适配表或学到的位置命中即可读取");
    }

    #[test]
    fn only_builds_older_than_the_table_ask_users_to_update(){
        assert!(older_than_table(&KUGOU,0x6ac9ec86-1));
        assert!(!older_than_table(&KUGOU,0x6ac9ec86+1),"比已适配的新：是岛仔还没适配，不让用户更新");
        assert!(!older_than_table(&NETEASE,0x6ae94a6a),"#202 里网易云 3.1.43 比适配表新");
    }

    #[test]
    fn table_rows_and_learned_offsets_both_resolve_a_layout(){
        assert_eq!(layout_for(&KUGOU,0x6ac9ec86,4587520),Some(Layout{position:0x3949f8,duration:None,learned:false}));
        assert_eq!(layout_for(&NETEASE,0x6ad712e1,41054208).unwrap().duration,Some(0x2214778));
        assert_eq!(layout_for(&KUGOU,1,2),None);
        compat::remember("kgplayer.dll",1,2,0x40);
        assert_eq!(layout_for(&KUGOU,1,2),Some(Layout{position:0x40,duration:None,learned:true}));
        compat::forget("kgplayer.dll",1,2);
    }

    fn bytes(values:&[f64])->Vec<u8>{values.iter().flat_map(|v|v.to_le_bytes()).collect()}

    #[test]
    fn learning_keeps_only_values_that_advance_with_real_time(){
        // 偏移 8 是进度；0 不变、16 跳太多、24 是计时但已超过一小时、32 是毫秒计数。
        let old=bytes(&[5.0,12.0,1.0,7200.0,12000.0]);
        let new=bytes(&[5.0,12.8,9.0,7200.8,12800.0]);
        assert_eq!(advancing(&old,&new,0.8),vec![(8,12.8)]);
        assert!(advanced(30.0,31.1,1.0),"允许播放器更新读数的时间误差");
        assert!(!advanced(30.0,30.0,1.0),"暂停或静止的数不是进度");
        assert!(!advanced(f64::NAN,1.0,1.0));
    }

    #[test]
    fn learning_decides_after_consistent_rounds(){
        assert!(matches!(decide(&[],1),Learn::Failed(_)));
        assert!(matches!(decide(&[(16,10.0)],2),Learn::Pending),"轮数不够先等");
        assert!(matches!(decide(&[(32,10.0),(16,10.2)],3),Learn::Found(16)),"同一进度的多个副本取偏移最小的");
        // 酷狗实测：0x3949f0 是领先约 8.7 秒的解码位置，0x3949f8 才是播放进度。
        assert!(matches!(decide(&[(0x3949f0,162.9),(0x3949f8,154.2)],3),Learn::Found(0x3949f8)),"同步增长的其他计时只会领先，取读数最小的");
        let many:Vec<_>=(0..9).map(|i|(i*8,10.0)).collect();
        assert!(matches!(decide(&many,3),Learn::Failed(_)),"候选太多不猜");
    }

    #[test]
    fn learned_layout_is_dropped_when_it_stops_advancing_while_playing(){
        let mut slot=Slot::default();
        assert!(!invalid_learned(&mut slot,Some(10.0),true));
        slot.stuck=Some((10.0,Instant::now()-STUCK));
        assert!(invalid_learned(&mut slot,Some(10.0),true),"播放中读数长时间不变");
        assert!(!invalid_learned(&mut slot,Some(10.0),false),"暂停时不变是正常的");
        assert!(!invalid_learned(&mut slot,Some(11.0),true));
        assert!(invalid_learned(&mut slot,None,true),"播放中读数无效");
    }

    /// 在本测试进程里模拟一个「播放组件」：数据区里有一个按真实时间增长的秒数。
    #[test]
    fn learner_finds_a_live_progress_value_in_a_real_process(){
        use std::sync::atomic::{AtomicBool,Ordering};
        static RUN:AtomicBool=AtomicBool::new(true);
        let data:&'static mut [f64;64]=Box::leak(Box::new([0.0;64]));
        data[0]=99999.0;data[7]=3.0;
        let ptr=data.as_mut_ptr() as usize;
        let started=Instant::now();
        let ticker=std::thread::spawn(move||while RUN.load(Ordering::Relaxed){
            unsafe{*(ptr as *mut f64).add(21)=1.5+started.elapsed().as_secs_f64();}
            std::thread::sleep(Duration::from_millis(50));
        });
        let me=std::env::current_exe().unwrap();let exe=me.file_name().unwrap().to_str().unwrap().to_owned();
        let p=Process::open(std::process::id(),&exe).expect("能读取自己的进程");
        let mut learner=Learner::new(std::process::id(),ptr,0,0,0);
        learner.sections=Some(vec![(0,64*8)]);
        let mut result=None;
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(750));
            match learner.step(&p,true,Instant::now()) {Learn::Pending=>{},other=>{result=Some(other);break;}}
        }
        RUN.store(false,Ordering::Relaxed);ticker.join().unwrap();
        assert!(matches!(result,Some(Learn::Found(offset)) if offset==21*8),"应找到偏移 168");
    }

    #[test]
    #[ignore="需要本机酷狗正在播放：打印组件数据区与已验证进度位置的关系，排查自动适配"]
    fn live_kugou_learning_diagnostics(){
        let (pid,base,stamp,size)=discover_in(KUGOU.exe,KUGOU.module,|_,_|None).err().and_then(|miss|miss.unknown).expect("需要酷狗正在运行");
        let p=Process::open(pid,KUGOU.exe).unwrap();
        let sections=writable_sections(p.0,base).unwrap();
        println!("可写区 {:?}",sections.iter().map(|(r,l)|format!("{r:#x}+{l:#x}")).collect::<Vec<_>>());
        let known=layout_for(&KUGOU,stamp,size).map(|l|l.position);
        println!("已验证位置 {known:x?}，在可写区内：{:?}",known.map(|k|sections.iter().any(|(r,l)|k>=*r && k<r+l)));
        for _ in 0..4 {
            let v=known.and_then(|k|read_f64(p.0,base+k));
            println!("已验证位置读数 {v:?}");
            std::thread::sleep(Duration::from_millis(800));
        }
        let mut learner=Learner::new(pid,base,stamp,size,0);
        for i in 0..12 {
            std::thread::sleep(Duration::from_millis(800));
            let r=learner.step(&p,true,Instant::now());
            let has=known.is_some_and(|k|learner.candidates.iter().any(|(o,_)|*o==k));
            println!("第{i}步：候选 {:x?}，含已验证位置 {has}，结果 {}",learner.candidates,match r{Learn::Pending=>"等待".into(),Learn::Found(o)=>format!("找到 {o:#x}"),Learn::Failed(e)=>format!("失败 {e}")});
            if !matches!(r,Learn::Pending){break;}
        }
    }

    #[test]
    #[ignore="需要本机酷狗正在播放：不用适配表，只靠自动适配找进度位置，应与适配表里的位置读数一致"]
    fn live_kugou_learning_matches_the_verified_offset(){
        let found=discover_in(KUGOU.exe,KUGOU.module,|_,_|None).err().and_then(|miss|miss.unknown).expect("需要酷狗正在运行");
        let (pid,base,stamp,size)=found;
        let p=Process::open(pid,KUGOU.exe).unwrap();
        let mut learner=Learner::new(pid,base,stamp,size,0);
        let started=Instant::now();
        let result=loop {
            std::thread::sleep(Duration::from_millis(800));
            match learner.step(&p,true,Instant::now()) {Learn::Pending=>{},other=>break other}
            assert!(started.elapsed()<Duration::from_secs(30),"30 秒内应有结论");
        };
        let Learn::Found(offset)=result else{panic!("自动适配失败")};
        let learned=read_f64(p.0,base+offset).unwrap();
        let verified=layout_for(&KUGOU,stamp,size).map(|l|read_f64(p.0,base+l.position).unwrap());
        println!("自动适配位置 {offset:#x}，读数 {learned:.2}；适配表读数 {verified:?}；用时 {:?}",started.elapsed());
        if let Some(verified)=verified {assert!((learned-verified).abs()<0.5,"与已验证位置读数一致");}
    }
}
