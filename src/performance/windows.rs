use super::{Process, Reading, Volume, LIST_LEN};
use ::windows::core::{w, PCWSTR};
use ::windows::Wdk::Graphics::Direct3D::*;
use ::windows::Wdk::System::SystemInformation::{NtQuerySystemInformation, SYSTEM_INFORMATION_CLASS};
use ::windows::Win32::{
    Foundation::{FILETIME, LUID},
    Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE},
    NetworkManagement::{
        IpHelper::{FreeMibTable, GetIfTable2},
        Ndis::IfOperStatusUp,
    },
    Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives},
    System::{
        Performance::*,
        Power::{CallNtPowerInformation, ProcessorInformation},
        ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION},
        SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
        Threading::GetSystemTimes,
    },
};
use std::collections::HashMap;
use std::time::{Duration, Instant};

type Interfaces = HashMap<u64, [u64; 2]>;

#[derive(Default)]
pub(super) struct Sampler {
    cpu: Option<[u64; 2]>,
    cores: Option<Vec<[i64; 3]>>,
    max_mhz: Option<Option<f32>>,
    net: Option<(Instant, Interfaces)>,
    disk: Option<Counters>,
    gpu: Option<Gpu>,
    gpu_retry: Option<Instant>,
    /// 上一次的程序处理器时间（按进程号和创建时间区分），只在展开页打开时保留。
    processes: Option<(Instant, HashMap<(usize, i64), (i64, u64)>)>,
    process_buffer: Vec<u64>,
    /// 程序图标，按小写名称缓存；取不到也记下，免得每秒重试。
    icons: HashMap<String, Option<std::sync::Arc<eframe::egui::ColorImage>>>,
    volumes: Option<(Instant, Vec<Volume>)>,
}

impl Sampler {
    pub fn read(&mut self, now: Instant, detail: bool) -> Reading {
        let mut r = Reading::default();
        let times = cpu_times();
        r.cpu = self.cpu.zip(times).and_then(|(prev, cur)| cpu_load(prev, cur));
        self.cpu = times;
        // 逐核读数很便宜，常驻的「核心点阵」也要用，所以一直采。
        let cores = core_times();
        if let (Some(prev), Some(cur)) = (&self.cores, &cores) {
            if prev.len() == cur.len() {
                (r.cores, r.cpu_kernel) = core_loads(prev, cur);
            }
        }
        self.cores = cores;
        let mut mem = MEMORYSTATUSEX {
            dwLength: size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        if unsafe { GlobalMemoryStatusEx(&mut mem) }.is_ok() {
            r.memory = Some((mem.ullTotalPhys.saturating_sub(mem.ullAvailPhys), mem.ullTotalPhys));
        }
        if self.gpu.is_none() && self.gpu_retry.is_none_or(|t| now >= t) {
            self.gpu = Gpu::open();
            self.gpu_retry = Some(now + Duration::from_secs(30));
        }
        if let Some(gpu) = &mut self.gpu {
            if !gpu.read(now, &mut r) {
                self.gpu = None;
            }
        }
        let next = interfaces();
        if let (Some((prev_at, prev)), Some((cur, _))) = (&self.net, &next) {
            r.network = net_rates(prev, cur, now.duration_since(*prev_at).as_secs_f64());
        }
        if let Some((cur, speed)) = &next {
            r.link_speed = (*speed > 0).then_some(*speed);
            r.net_total = Some(cur.values().fold([0, 0], |t, v| [t[0] + v[0], t[1] + v[1]]));
        }
        self.net = next.map(|(v, _)| (now, v));
        if self.disk.is_none() {
            self.disk = Counters::open();
        }
        if let Some(counters) = &self.disk {
            let [read, write, idle, performance, free] = counters.read();
            r.memory_free = free.map(|bytes| bytes as u64);
            r.disk = [read, write];
            r.disk_active = idle.map(|v| (100. - v).clamp(0., 100.));
            let max = *self.max_mhz.get_or_insert_with(max_mhz);
            r.cpu_mhz = max.zip(performance).map(|(mhz, pct)| mhz * pct / 100.).filter(|v| *v > 0.);
        }
        if detail {
            self.read_detail(now, &mut r);
        } else {
            self.processes = None;
        }
        r
    }

    /// 展开页才需要的统计：程序排行、内存构成、分区容量。
    fn read_detail(&mut self, now: Instant, r: &mut Reading) {
        let mut info = PERFORMANCE_INFORMATION {
            cb: size_of::<PERFORMANCE_INFORMATION>() as u32,
            ..Default::default()
        };
        if unsafe { GetPerformanceInfo(&mut info, info.cb) }.is_ok() {
            let page = info.PageSize as u64;
            r.memory_cache = Some(info.SystemCache as u64 * page);
            r.memory_kernel = Some(info.KernelNonpaged as u64 * page);
        }
        if let Some(table) = process_table(&mut self.process_buffer) {
            let threads = self.cores.as_ref().map_or(1, |c| c.len().max(1)) as f64;
            let elapsed = self.processes.as_ref().map(|(at, _)| now.duration_since(*at).as_secs_f64());
            let mut programs: HashMap<String, Process> = HashMap::new();
            let mut biggest: HashMap<String, u64> = HashMap::new();
            let mut times = HashMap::with_capacity(table.len());
            for entry in table {
                let (cpu, io) = match (&self.processes, elapsed) {
                    (Some((_, prev)), Some(secs)) if secs > 0. && secs <= 5. => match prev.get(&(entry.pid, entry.created)) {
                        Some((cpu, io)) => (
                            entry.cpu.checked_sub(*cpu).map_or(0., |ticks| (ticks as f64 / (secs * 1e7 * threads) * 100.) as f32),
                            entry.io.checked_sub(*io).map_or(0., |bytes| (bytes as f64 / secs) as f32),
                        ),
                        None => (0., 0.),
                    },
                    _ => (0., 0.),
                };
                times.insert((entry.pid, entry.created), (entry.cpu, entry.io));
                let program = programs.entry(entry.name.to_lowercase()).or_insert_with(|| Process {
                    name: entry.name.clone(),
                    ..Default::default()
                });
                program.cpu += cpu;
                program.io += io;
                if program.pid == 0 || entry.memory > top_memory(&biggest, &entry.name) {
                    program.pid = entry.pid;
                    biggest.insert(entry.name.to_lowercase(), entry.memory);
                }
                program.memory += entry.memory;
            }
            let warm = elapsed.is_some();
            self.processes = Some((now, times));
            let mut programs: Vec<_> = programs.into_values().filter(|p| !p.name.is_empty()).collect();
            programs.sort_by(|a, b| b.memory.cmp(&a.memory));
            r.by_memory = programs.iter().take(LIST_LEN).cloned().collect();
            if warm {
                programs.sort_by(|a, b| b.io.total_cmp(&a.io));
                r.by_io = programs.iter().take(5).filter(|p| p.io > 0.).cloned().collect();
                programs.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
                r.by_cpu = programs.into_iter().take(LIST_LEN).filter(|p| p.cpu > 0.).collect();
            }
            if self.icons.len() > 256 {
                self.icons.clear();
            }
            for program in r.by_memory.iter_mut().chain(r.by_cpu.iter_mut()).chain(r.by_io.iter_mut()) {
                program.icon = self.icons.entry(program.name.to_lowercase()).or_insert_with(|| program_icon(program.pid)).clone();
            }
        }
        if self.volumes.as_ref().is_none_or(|(at, _)| now.duration_since(*at) >= Duration::from_secs(15)) {
            self.volumes = Some((now, volumes()));
        }
        r.volumes = self.volumes.as_ref().map(|(_, v)| v.clone()).unwrap_or_default();
    }
}

/// `SystemProcessorPerformanceInformation` 的一项：每个逻辑处理器一份。
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ProcessorTimes {
    idle: i64,
    kernel: i64,
    user: i64,
    dpc: i64,
    interrupt: i64,
    count: u32,
}

fn core_times() -> Option<Vec<[i64; 3]>> {
    let mut buffer = vec![ProcessorTimes::default(); 256];
    let mut len = 0u32;
    let status = unsafe {
        NtQuerySystemInformation(
            SYSTEM_INFORMATION_CLASS(8),
            buffer.as_mut_ptr().cast(),
            (buffer.len() * size_of::<ProcessorTimes>()) as u32,
            &mut len,
        )
    };
    if status.0 < 0 {
        return None;
    }
    buffer.truncate(len as usize / size_of::<ProcessorTimes>());
    (!buffer.is_empty()).then(|| buffer.iter().map(|t| [t.idle, t.kernel, t.user]).collect())
}

/// 逐核占用率，以及内核态（内核时间减去空闲）占全部处理器时间的比例。
fn core_loads(prev: &[[i64; 3]], cur: &[[i64; 3]]) -> (Vec<f32>, Option<f32>) {
    let (mut kernel_busy, mut all) = (0i64, 0i64);
    let loads = prev
        .iter()
        .zip(cur)
        .map(|(a, b)| {
            let idle = (b[0] - a[0]).max(0);
            let kernel = (b[1] - a[1]).max(0);
            let total = kernel + (b[2] - a[2]).max(0);
            kernel_busy += (kernel - idle).max(0);
            all += total;
            if total > 0 { (100. * (total - idle).max(0) as f64 / total as f64) as f32 } else { 0. }
        })
        .collect();
    (loads, (all > 0).then(|| (100. * kernel_busy as f64 / all as f64) as f32))
}

fn max_mhz() -> Option<f32> {
    let count = std::thread::available_parallelism().map_or(1, |n| n.get()).max(1) * 2;
    let mut info = vec![[0u32; 6]; count];
    let status = unsafe {
        CallNtPowerInformation(
            ProcessorInformation,
            None,
            0,
            Some(info.as_mut_ptr().cast()),
            (info.len() * size_of::<[u32; 6]>()) as u32,
        )
    };
    if status.0 < 0 {
        return None;
    }
    info.iter().map(|p| p[1]).max().filter(|m| *m > 0).map(|m| m as f32)
}

fn top_memory(biggest: &HashMap<String, u64>, name: &str) -> u64 {
    biggest.get(&name.to_lowercase()).copied().unwrap_or(0)
}

/// 由进程号找到程序文件，取它的图标。系统进程打不开或没有文件时返回空。
fn program_icon(pid: usize) -> Option<std::sync::Arc<eframe::egui::ColorImage>> {
    use ::windows::Win32::Foundation::CloseHandle;
    use ::windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
    if pid <= 4 {
        return None;
    }
    let path = unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid as u32).ok()?;
        let mut name = [0u16; 1024];
        let mut length = name.len() as u32;
        let result = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, ::windows::core::PWSTR(name.as_mut_ptr()), &mut length);
        let _ = CloseHandle(process);
        result.ok()?;
        std::path::PathBuf::from(String::from_utf16_lossy(&name[..length as usize]))
    };
    crate::icons::shell_icon(&path).map(std::sync::Arc::new)
}

struct ProcessEntry {
    pid: usize,
    created: i64,
    cpu: i64,
    memory: u64,
    /// 累计读写字节数（ReadTransferCount + WriteTransferCount）。
    io: u64,
    name: String,
}

/// 一次系统调用取回全部进程的名称、处理器时间和私有工作集，不必逐个打开进程。
#[cfg(target_pointer_width = "64")]
fn process_table(buffer: &mut Vec<u64>) -> Option<Vec<ProcessEntry>> {
    const MISMATCH: i32 = 0xC000_0004_u32 as i32;
    if buffer.is_empty() {
        buffer.resize(64 * 1024, 0);
    }
    let mut len = 0u32;
    let mut done = false;
    for _ in 0..4 {
        let status = unsafe {
            NtQuerySystemInformation(SYSTEM_INFORMATION_CLASS(5), buffer.as_mut_ptr().cast(), (buffer.len() * 8) as u32, &mut len)
        };
        if status.0 == MISMATCH {
            buffer.resize((len as usize / 8 + 8 * 1024).max(buffer.len() * 2), 0);
            continue;
        }
        if status.0 < 0 {
            return None;
        }
        done = true;
        break;
    }
    if !done {
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), (len as usize).min(buffer.len() * 8)) };
    // SYSTEM_PROCESS_INFORMATION（x64）里用到的字段偏移。
    let read = |at: usize, offset: usize, width: usize| -> Option<u64> {
        let slice = bytes.get(at + offset..at + offset + width)?;
        let mut raw = [0u8; 8];
        raw[..width].copy_from_slice(slice);
        Some(u64::from_le_bytes(raw))
    };
    let mut entries = Vec::with_capacity(512);
    let mut at = 0usize;
    loop {
        let next = read(at, 0, 4)? as usize;
        let pid = read(at, 80, 8)? as usize;
        if pid != 0 {
            let chars = read(at, 56, 2)? as usize / 2;
            let pointer = read(at, 64, 8)? as usize as *const u16;
            let start = bytes.as_ptr() as usize;
            let inside = (pointer as usize) >= start && (pointer as usize) + chars * 2 <= start + bytes.len();
            let name = if !inside || chars == 0 {
                String::new()
            } else {
                // 名称指针指向同一块缓冲区内部，上面已核对范围。
                String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(pointer, chars) })
            };
            let name = match name.len().checked_sub(4) {
                Some(cut) if name.get(cut..).is_some_and(|ext| ext.eq_ignore_ascii_case(".exe")) => name[..cut].to_string(),
                _ => name,
            };
            entries.push(ProcessEntry {
                pid,
                created: read(at, 32, 8)? as i64,
                cpu: read(at, 40, 8)? as i64 + read(at, 48, 8)? as i64,
                memory: read(at, 8, 8)?,
                io: read(at, 0xE8, 8)? + read(at, 0xF0, 8)?,
                name,
            });
        }
        if next == 0 {
            break;
        }
        at += next;
    }
    Some(entries)
}
#[cfg(not(target_pointer_width = "64"))]
fn process_table(_: &mut Vec<u64>) -> Option<Vec<ProcessEntry>> {
    None
}

fn volumes() -> Vec<Volume> {
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|i| mask & (1 << i) != 0)
        .filter_map(|i| {
            let letter = (b'A' + i) as char;
            let root: Vec<u16> = format!("{letter}:\\").encode_utf16().chain([0]).collect();
            // 只统计本地固定磁盘（DRIVE_FIXED = 3），不碰网络盘和光驱。
            if unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) } != 3 {
                return None;
            }
            let (mut total, mut free) = (0u64, 0u64);
            unsafe { GetDiskFreeSpaceExW(PCWSTR(root.as_ptr()), None, Some(&mut total), Some(&mut free)) }.ok()?;
            (total > 0).then(|| Volume { letter, used: total.saturating_sub(free), total })
        })
        .collect()
}

fn cpu_times() -> Option<[u64; 2]> {
    let (mut idle, mut kernel, mut user) = Default::default();
    unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.ok()?;
    let ticks = |v: FILETIME| u64::from(v.dwHighDateTime) << 32 | u64::from(v.dwLowDateTime);
    Some([ticks(idle), ticks(kernel) + ticks(user)])
}
fn cpu_load(prev: [u64; 2], cur: [u64; 2]) -> Option<f32> {
    let total = cur[1].checked_sub(prev[1])?;
    let busy = total.checked_sub(cur[0].checked_sub(prev[0])?)?;
    (total > 0).then(|| 100. * busy as f32 / total as f32)
}

fn interfaces() -> Option<(Interfaces, u64)> {
    let mut table = std::ptr::null_mut();
    unsafe { GetIfTable2(&mut table) }.ok().ok()?;
    // 行数由同一次系统分配给出，只在释放表前借用行。
    let rows = unsafe { std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize) };
    let mut counters = HashMap::new();
    let mut speed = 0u64;
    for row in rows {
        if row.OperStatus == IfOperStatusUp && row.InterfaceAndOperStatusFlags._bitfield & 3 == 1 && row.Type != 24 {
            counters.insert(unsafe { row.InterfaceLuid.Value }, [row.InOctets, row.OutOctets]);
            if row.ReceiveLinkSpeed < u64::MAX {
                speed = speed.max(row.ReceiveLinkSpeed);
            }
        }
    }
    unsafe { FreeMibTable(table.cast()) };
    Some((counters, speed))
}

fn net_rates(prev: &Interfaces, cur: &Interfaces, secs: f64) -> [Option<f32>; 2] {
    if secs <= 0. || secs > 5. {
        return [None; 2];
    }
    std::array::from_fn(|i| {
        let mut changes = cur
            .iter()
            .filter_map(|(key, bytes)| bytes[i].checked_sub(prev.get(key)?[i]))
            .peekable();
        if changes.peek().is_none() {
            return None;
        }
        Some((changes.map(|n| n as f64).sum::<f64>() / secs) as f32)
    })
}

/// 同一个 PDH 查询里的四个计数器：磁盘读、写、空闲时间，处理器性能百分比（换算当前频率）。
struct Counters {
    query: isize,
    counters: [isize; 5],
}
impl Counters {
    fn open() -> Option<Self> {
        let mut disk = Self {
            query: 0,
            counters: [0; 5],
        };
        if unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut disk.query) } != 0 {
            return None;
        }
        let names = [
            w!("\\PhysicalDisk(_Total)\\Disk Read Bytes/sec"),
            w!("\\PhysicalDisk(_Total)\\Disk Write Bytes/sec"),
            w!("\\PhysicalDisk(_Total)\\% Idle Time"),
            w!("\\Processor Information(_Total)\\% Processor Performance"),
            // 真正空着的内存（空闲 + 已清零页）；可用内存减去它就是随时能让出来的备用缓存。
            w!("\\Memory\\Free & Zero Page List Bytes"),
        ];
        for (slot, name) in disk.counters.iter_mut().zip(names) {
            unsafe { PdhAddEnglishCounterW(disk.query, name, 0, slot) };
        }
        Some(disk)
    }
    fn read(&self) -> [Option<f32>; 5] {
        if unsafe { PdhCollectQueryData(self.query) } != 0 {
            return [None; 5];
        }
        self.counters.map(|c| {
            let mut v = PDH_FMT_COUNTERVALUE::default();
            if c == 0 || unsafe { PdhGetFormattedCounterValue(c, PDH_FMT_DOUBLE, None, &mut v) } != 0 || v.CStatus > 1 {
                return None;
            }
            let rate = unsafe { v.Anonymous.doubleValue } as f32;
            (rate.is_finite() && rate >= 0.).then_some(rate)
        })
    }
}
impl Drop for Counters {
    fn drop(&mut self) {
        if self.query != 0 {
            unsafe { PdhCloseQuery(self.query) };
        }
    }
}

struct Gpu {
    luid: LUID,
    handle: Option<u32>,
    nodes: u32,
    segments: u32,
    capacity: u64,
    shared: u64,
    /// 每个节点的引擎类别（取不到元数据时为空）。
    engines: Vec<Option<&'static str>>,
    name: String,
    previous: Option<(Instant, Vec<i64>)>,
}

impl Gpu {
    fn open() -> Option<Self> {
        let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.ok()?;
        let desc = (0..)
            .map_while(|i| unsafe { factory.EnumAdapters1(i) }.ok())
            .filter_map(|a| unsafe { a.GetDesc1() }.ok())
            .filter(|d| d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 == 0)
            .max_by_key(|d| d.DedicatedVideoMemory)?;
        let name = String::from_utf16_lossy(
            &desc.Description[..desc
                .Description
                .iter()
                .position(|&n| n == 0)
                .unwrap_or(desc.Description.len())],
        );
        let mut gpu = Self {
            luid: desc.AdapterLuid,
            handle: None,
            nodes: 0,
            segments: 0,
            capacity: desc.DedicatedVideoMemory as u64,
            shared: desc.SharedSystemMemory as u64,
            engines: Vec::new(),
            name,
            previous: None,
        };
        let info = unsafe { gpu.stat(D3DKMT_QUERYSTATISTICS_ADAPTER, 0)?.AdapterInformation };
        gpu.nodes = info.NodeCount;
        gpu.segments = info.NbSegments;
        let mut open = D3DKMT_OPENADAPTERFROMLUID {
            AdapterLuid: gpu.luid,
            hAdapter: 0,
        };
        if unsafe { D3DKMTOpenAdapterFromLuid(&mut open) }.is_ok() {
            gpu.handle = Some(open.hAdapter);
            gpu.engines = (0..gpu.nodes).map(|i| gpu.engine(i)).collect();
        }
        Some(gpu)
    }
    /// 节点的引擎类别，按任务管理器的分法起中文名；不常见的类别不单独列出。
    fn engine(&self, node: u32) -> Option<&'static str> {
        let mut meta = D3DKMT_NODEMETADATA { NodeOrdinalAndAdapterIndex: node, ..Default::default() };
        let mut q = D3DKMT_QUERYADAPTERINFO {
            hAdapter: self.handle?,
            Type: KMTQAITYPE_NODEMETADATA,
            pPrivateDriverData: (&raw mut meta).cast(),
            PrivateDriverDataSize: size_of_val(&meta) as u32,
        };
        unsafe { D3DKMTQueryAdapterInfo(&mut q) }.is_ok().then_some(())?;
        // 结构体按 1 字节对齐，先把字段拷出来再用。
        let (kind, name) = (meta.NodeData.EngineType, meta.NodeData.FriendlyName);
        let friendly = String::from_utf16_lossy(&name[..name.iter().position(|&n| n == 0).unwrap_or(name.len())]);
        match kind {
            DXGK_ENGINE_TYPE_3D => Some("3D"),
            DXGK_ENGINE_TYPE_VIDEO_DECODE => Some("视频解码"),
            DXGK_ENGINE_TYPE_VIDEO_ENCODE => Some("视频编码"),
            DXGK_ENGINE_TYPE_VIDEO_PROCESSING => Some("视频处理"),
            DXGK_ENGINE_TYPE_COPY => Some("复制"),
            DXGK_ENGINE_TYPE_OTHER if friendly.to_lowercase().starts_with("compute") => Some("计算"),
            _ => None,
        }
    }
    fn stat(&self, kind: D3DKMT_QUERYSTATISTICS_TYPE, index: u32) -> Option<D3DKMT_QUERYSTATISTICS_RESULT> {
        let mut q = D3DKMT_QUERYSTATISTICS {
            Type: kind,
            AdapterLuid: self.luid,
            ..Default::default()
        };
        if kind == D3DKMT_QUERYSTATISTICS_NODE {
            q.Anonymous.QueryNode.NodeId = index;
        }
        if kind == D3DKMT_QUERYSTATISTICS_SEGMENT {
            q.Anonymous.QuerySegment.SegmentId = index;
        }
        // 内核根据 Type 写入对应的联合体成员。
        unsafe { D3DKMTQueryStatistics(&raw mut q) }.is_ok().then_some(q.QueryResult)
    }
    fn read(&mut self, now: Instant, r: &mut Reading) -> bool {
        let times: Option<Vec<_>> = (0..self.nodes)
            .map(|i| {
                Some(unsafe {
                    self.stat(D3DKMT_QUERYSTATISTICS_NODE, i)?
                        .NodeInformation
                        .GlobalInformation
                        .RunningTime
                })
            })
            .collect();
        let Some(times) = times else {
            return false;
        };
        if let Some((at, prev)) = &self.previous {
            let interval = now.duration_since(*at).as_secs_f64();
            if interval > 0. && interval <= 5. {
                let loads: Vec<Option<f32>> = times
                    .iter()
                    .zip(prev)
                    .map(|(a, b)| a.checked_sub(*b).filter(|n| *n >= 0).map(|ticks| (ticks as f64 / (1e5 * interval)).clamp(0., 100.) as f32))
                    .collect();
                r.gpu = loads.iter().flatten().copied().reduce(f32::max);
                // 同类引擎取最忙的节点，按第一次出现的顺序排。
                for (load, kind) in loads.iter().zip(&self.engines) {
                    let (Some(load), Some(kind)) = (load, kind) else { continue };
                    match r.gpu_engines.iter_mut().find(|(k, _)| k == kind) {
                        Some((_, v)) => *v = v.max(*load),
                        None => r.gpu_engines.push((kind, *load)),
                    }
                }
            }
        }
        self.previous = Some((now, times));
        r.gpu_name = self.name.clone();
        if self.capacity > 0 {
            let used = (0..self.segments)
                .map(|i| Some(unsafe { self.stat(D3DKMT_QUERYSTATISTICS_SEGMENT, i)?.SegmentInformation }))
                .try_fold(0u64, |total, seg| {
                    let s = seg?;
                    Some(total + if s.Aperture == 0 { s.BytesResident } else { 0 })
                });
            r.video_memory = used.map(|v| (v, self.capacity));
        }
        if self.shared > 0 {
            // 共享段（Aperture）里常驻的字节，就是任务管理器里的「共享 GPU 内存」。
            let used = (0..self.segments)
                .map(|i| Some(unsafe { self.stat(D3DKMT_QUERYSTATISTICS_SEGMENT, i)?.SegmentInformation }))
                .try_fold(0u64, |total, seg| {
                    let s = seg?;
                    Some(total + if s.Aperture != 0 { s.BytesResident } else { 0 })
                });
            r.shared_memory = used.map(|v| (v, self.shared));
        }
        if let Some(handle) = self.handle {
            let mut perf = D3DKMT_ADAPTER_PERFDATA::default();
            let mut q = D3DKMT_QUERYADAPTERINFO {
                hAdapter: handle,
                Type: KMTQAITYPE_ADAPTERPERFDATA,
                pPrivateDriverData: (&raw mut perf).cast(),
                PrivateDriverDataSize: size_of_val(&perf) as u32,
            };
            if unsafe { D3DKMTQueryAdapterInfo(&mut q) }.is_ok() && (1..1300).contains(&perf.Temperature) {
                r.gpu_temp = Some(perf.Temperature as f32 / 10.);
            }
        }
        true
    }
}
impl Drop for Gpu {
    fn drop(&mut self) {
        if let Some(h) = self.handle {
            let _ = unsafe { D3DKMTCloseAdapter(&D3DKMT_CLOSEADAPTER { hAdapter: h }) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counter_resets_and_adapter_changes_do_not_spike() {
        assert_eq!(cpu_load([20, 100], [40, 180]), Some(75.));
        assert_eq!(cpu_load([20, 100], [1, 10]), None);
        let prev = HashMap::from([(1, [100, 90]), (2, [800, 900])]);
        let cur = HashMap::from([(1, [300, 40]), (3, [5000, 9000])]);
        assert_eq!(net_rates(&prev, &cur, 2.), [Some(100.), None]);
        let (cores, kernel) = core_loads(&[[0, 0, 0], [0, 0, 0]], &[[50, 80, 20], [100, 100, 0]]);
        assert_eq!(cores, vec![50., 0.]);
        assert_eq!(kernel, Some(15.));
    }
    #[test]
    fn detail_sampling_lists_programs_and_volumes() {
        let mut sampler = Sampler::default();
        sampler.read(Instant::now(), true);
        std::thread::sleep(Duration::from_millis(1050));
        let r = sampler.read(Instant::now(), true);
        assert!(!r.cores.is_empty() && r.cores.iter().all(|v| (0.0..=100.0).contains(v)));
        assert!(!r.by_memory.is_empty() && r.by_memory.iter().all(|p| !p.name.is_empty() && p.memory > 0));
        assert!(r.by_memory.windows(2).all(|w| w[0].memory >= w[1].memory));
        assert!(r.volumes.iter().all(|v| v.used <= v.total) && !r.volumes.is_empty());
        assert!(r.memory_cache.is_some() && r.memory_kernel.is_some() && r.memory_free.is_some());
        println!("{:?} {:?} {:?} {:?} {:?}", r.cpu_mhz, r.cpu_kernel, r.disk_active, r.link_speed, &r.by_cpu);
        let quiet = sampler.read(Instant::now(), false);
        assert!(quiet.by_memory.is_empty() && quiet.volumes.is_empty(), "收起后不再统计程序和分区");
    }
    #[test]
    fn windows_samples_live_cpu_and_memory() {
        let mut sampler = Sampler::default();
        let first = sampler.read(Instant::now(), false);
        assert!(first.cpu.is_none());
        std::thread::sleep(Duration::from_millis(1050));
        let next = sampler.read(Instant::now(), false);
        assert!(next.cpu.is_some_and(|v| (0.0..=100.0).contains(&v)));
        assert!(next.memory.is_some_and(|(used, total)| used <= total && total > 0));
        println!("{next:?}");
    }
}
