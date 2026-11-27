//! Windows PnP 与 BLE 分开读取；慢 GATT 请求不能阻塞连接状态和系统电量。
use super::{device_kind,state::*};
use std::{sync::{Arc,atomic::{AtomicBool,Ordering},mpsc::{self,SyncSender}},time::{Duration,Instant}};
use windows::{core::{GUID,HSTRING,PCWSTR,RuntimeType},Foundation::{AsyncStatus,IAsyncOperation},Devices::{Bluetooth::{BluetoothCacheMode,BluetoothConnectionStatus,BluetoothDevice,BluetoothLEDevice,GenericAttributeProfile::{GattCommunicationStatus,GattDeviceService}},Enumeration::DeviceInformation},Storage::Streams::DataReader};
use windows::Win32::{Devices::{DeviceAndDriverInstallation::*,Properties::*},Foundation::HWND,System::WinRT::{RoInitialize,RoUninitialize,RO_INIT_MULTITHREADED}};

struct Apartment;
impl Apartment {fn new()->Option<Self>{unsafe {RoInitialize(RO_INIT_MULTITHREADED).ok()?;}Some(Self)}}
impl Drop for Apartment {fn drop(&mut self){unsafe {RoUninitialize();}}}

fn wait<T:RuntimeType+'static>(op:windows::core::Result<IAsyncOperation<T>>,stop:&AtomicBool)->Option<T> {
    let op=op.ok()?;let start=Instant::now();
    loop {
        if stop.load(Ordering::Relaxed)||start.elapsed()>Duration::from_secs(3){let _=op.Cancel();return None;}
        match op.Status().ok()? {
            AsyncStatus::Completed=>return op.GetResults().ok(),
            AsyncStatus::Started=>std::thread::sleep(Duration::from_millis(40)),
            _=>return None,
        }
    }
}

#[derive(Clone)]
struct PnpNode {address:String,container:Option<String>,battery:Option<u8>}
struct DeviceSet(HDEVINFO);
impl Drop for DeviceSet {fn drop(&mut self){unsafe {let _=SetupDiDestroyDeviceInfoList(self.0);}}}

fn property(set:HDEVINFO,data:&SP_DEVINFO_DATA,key:&DEVPROPKEY)->Option<(DEVPROPTYPE,Vec<u8>)> {
    let mut kind=DEVPROPTYPE::default();let mut bytes=[0u8;512];let mut size=0;
    unsafe {SetupDiGetDevicePropertyW(set,data,key,&mut kind,Some(&mut bytes),Some(&mut size),0).ok()?;}
    Some((kind,bytes.get(..size as usize)?.to_vec()))
}
fn pnp_nodes()->Option<Vec<PnpNode>> {
    const BATTERY:DEVPROPKEY=DEVPROPKEY{fmtid:GUID::from_u128(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5),pid:2};
    let set=DeviceSet(unsafe {SetupDiGetClassDevsW(None,PCWSTR::null(),HWND::default(),DIGCF_PRESENT|DIGCF_ALLCLASSES).ok()?});
    let mut nodes=vec![];
    for i in 0..u32::MAX {
        let mut data=SP_DEVINFO_DATA{cbSize:std::mem::size_of::<SP_DEVINFO_DATA>() as u32,..Default::default()};
        if unsafe {SetupDiEnumDeviceInfo(set.0,i,&mut data)}.is_err(){break;}
        let mut text=[0u16;512];let mut size=0;
        if unsafe {SetupDiGetDeviceInstanceIdW(set.0,&data,Some(&mut text),Some(&mut size))}.is_err(){continue;}
        let instance=String::from_utf16_lossy(&text[..(size as usize).saturating_sub(1).min(text.len())]);
        let Some(address)=pnp_address(&instance) else {continue};
        let root = instance.split('\\').nth(1).is_some_and(|s| s.to_ascii_uppercase().starts_with("DEV_"));
        let container=root.then(||property(set.0,&data,&DEVPKEY_Device_ContainerId)).flatten().and_then(|(ty,bytes)|{
            (ty==DEVPROP_TYPE_GUID&&bytes.len()==16&&bytes.iter().any(|b|*b!=0)).then(||bytes.iter().map(|b|format!("{b:02X}")).collect())
        });
        let battery=property(set.0,&data,&BATTERY).and_then(|(ty,bytes)|{
            (ty==DEVPROP_TYPE_BYTE&&bytes.len()==1).then(||bytes[0]).filter(|v|*v<=100)
        });
        nodes.push(PnpNode{address,container,battery});
    }
    Some(nodes)
}

fn discover(previous:&[Target],nodes:&[PnpNode],stop:&AtomicBool)->Option<Vec<Endpoint>> {
    let selectors=[(false,BluetoothDevice::GetDeviceSelectorFromConnectionStatus(BluetoothConnectionStatus::Connected).ok()?),
        (true,BluetoothLEDevice::GetDeviceSelectorFromConnectionStatus(BluetoothConnectionStatus::Connected).ok()?)];
    let mut found=vec![];
    for (ble,selector) in selectors {
        let selector=HSTRING::from(format!("({selector}) AND System.Devices.Aep.IsConnected:=System.StructuredQueryType.Boolean#True"));
        // 任何一个传输枚举失败，都不把不完整快照当作设备断开。
        let infos=wait(DeviceInformation::FindAllAsyncAqsFilter(&selector),stop)?;
        for index in 0..infos.Size().ok()? {
            let Ok(info)=infos.GetAt(index) else {continue};
            let Ok(id)=info.Id() else {continue};
            let native_id=id.to_string();
            let cached=previous.iter().find(|t|t.endpoint.id==native_id);
            let name=info.Name().ok().map(|s|s.to_string()).filter(|s|!s.trim().is_empty())
                .or_else(||cached.map(|t|t.endpoint.name.clone()));
            let Some(name)=name else {continue};
            let (address,kind)=if let Some(old)=cached.filter(|t|t.endpoint.address.is_some()) {(old.endpoint.address.clone(),old.endpoint.kind)}else{
                let (address,class)=if ble {
                    if let Some(device)=wait(BluetoothLEDevice::FromIdAsync(&id),stop){
                        let result=(device.BluetoothAddress().ok(),device.Appearance().ok().and_then(|v|v.RawValue().ok()).map(u32::from));let _=device.Close();result
                    }else{(None,None)}
                }else if let Some(device)=wait(BluetoothDevice::FromIdAsync(&id),stop){
                    let result=(device.BluetoothAddress().ok(),device.ClassOfDevice().ok().and_then(|v|v.RawValue().ok()));let _=device.Close();result
                }else{(None,None)};
                let address=address.and_then(|a|normalize_address(&format!("{a:012X}"))).or_else(||normalize_address(native_id.rsplit('-').next().unwrap_or_default()));
                (address,device_kind(&name,!ble,class))
            };
            let container=address.as_ref().and_then(|a|nodes.iter().find(|n|&n.address==a&&n.container.is_some())).and_then(|n|n.container.clone());
            found.push(Endpoint{id:native_id,address,container,name,kind,ble});
        }
    }
    Some(found)
}

struct BleHandle(BluetoothLEDevice);
impl Drop for BleHandle {fn drop(&mut self){let _=self.0.Close();}}
struct GattService(GattDeviceService);
impl Drop for GattService {fn drop(&mut self){let _=self.0.Close();}}

// 每个服务独立读取。可选特征的访问错误只影响该特征。
fn characteristic_value(service: &GattDeviceService, uuid: u128, stop: &AtomicBool) -> Option<Vec<u8>> {
    let chars = wait(service.GetCharacteristicsForUuidWithCacheModeAsync(GUID::from_u128(uuid),BluetoothCacheMode::Cached), stop)?;
    if chars.Status().ok()? != GattCommunicationStatus::Success { return None; }
    let characteristic = chars.Characteristics().ok()?.GetAt(0).ok()?;
    let result = wait(characteristic.ReadValueWithCacheModeAsync(BluetoothCacheMode::Uncached), stop)?;
    if result.Status().ok()? != GattCommunicationStatus::Success { return None; }
    let buffer = result.Value().ok()?;
    let length = buffer.Length().ok()? as usize;
    if !(1..=64).contains(&length) { return None; }
    let mut bytes = vec![0; length];
    DataReader::FromBuffer(&buffer).ok()?.ReadBytes(&mut bytes).ok()?;
    Some(bytes)
}

fn read_gatt(target:&Target,stop:&AtomicBool)->Option<Sample> {
    let device=BleHandle(wait(BluetoothLEDevice::FromIdAsync(&HSTRING::from(&target.endpoint.id)),stop)?);
    if device.0.ConnectionStatus().ok()?!=BluetoothConnectionStatus::Connected {return None;}
    let services=wait(device.0.GetGattServicesForUuidWithCacheModeAsync(GUID::from_u128(0x0000180f_0000_1000_8000_00805f9b34fb),BluetoothCacheMode::Cached),stop)?;
    if services.Status().ok()?!=GattCommunicationStatus::Success {return None;}
    let services:Vec<_>=services.Services().ok()?.into_iter().map(GattService).collect();
    const LEVEL: u128 = 0x00002a19_0000_1000_8000_00805f9b34fb;
    const STATUS: u128 = 0x00002bed_0000_1000_8000_00805f9b34fb;
    let (percent, charging) = services.iter().filter_map(|service| {
        let level = characteristic_value(&service.0, LEVEL, stop).and_then(|bytes| match bytes.as_slice() {
            [v] if *v <= 100 => Some(*v),
            _ => None,
        });
        let status = characteristic_value(&service.0, STATUS, stop)
            .and_then(|bytes| super::bas::parse_status(&bytes));
        super::bas::service_level(level, status)
    }).min_by_key(|(percent, _)| *percent)?;
    Some(Sample {
        key: target.key.clone(), session: target.session,
        reading: BatteryReading::new(percent,BatterySource::Gatt,Instant::now(),charging),
    })
}

#[derive(Default)]
struct PollSchedule { last:Option<Instant>, targets:Vec<(String,u64)> }
impl PollSchedule {
    fn due(&self,now:Instant)->bool {
        self.last.is_none_or(|at|now.saturating_duration_since(at)>=Duration::from_secs(60))
    }
    fn begin_read(&mut self,now:Instant,targets:Vec<(String,u64)>)->bool {
        if !self.due(now)&&targets==self.targets {return false;}
        self.last=Some(now);
        self.targets=targets;
        true
    }
}

fn windows_samples(targets:&[Target],nodes:&[PnpNode],now:Instant)->Vec<Sample> {
    let mut levels=std::collections::BTreeMap::new();
    for target in targets {
        let Some(address)=&target.endpoint.address else {continue};
        if let Some(level)=nodes.iter().filter(|n|&n.address==address).filter_map(|n|n.battery).min() {
            let value=levels.entry((&target.key,target.session)).or_insert(level);
            *value=level.min(*value);
        }
    }
    levels.into_iter().map(|((key,session),percent)|Sample{key:key.clone(),session,
        reading:BatteryReading::new(percent,BatterySource::Windows,now,None)}).collect()
}

pub(super) fn run(tx:SyncSender<Vec<super::Device>>,stop:Arc<AtomicBool>) {
    let Some(_apartment)=Apartment::new() else {return};
    let (jobs,requests)=mpsc::sync_channel::<Vec<Target>>(1);
    let (results,readings)=mpsc::sync_channel::<Sample>(128);
    let gatt_stop=stop.clone();
    let worker=std::thread::Builder::new().name("daozai-bluetooth-gatt".into()).spawn(move||{
        let Some(_apartment)=Apartment::new() else {return};
        while !gatt_stop.load(Ordering::Relaxed){
            let Ok(mut targets)=requests.recv_timeout(Duration::from_millis(200)) else {continue};
            while let Ok(latest)=requests.try_recv(){targets=latest;}
            for target in targets.iter().filter(|t|t.endpoint.ble){
                if gatt_stop.load(Ordering::Relaxed){return;}
                if let Some(reading)=read_gatt(target,&gatt_stop){let _=results.try_send(reading);}
            }
        }
    }).ok();
    let mut state=State::default();let mut nodes=vec![];let mut schedule=PollSchedule::default();
    while !stop.load(Ordering::Relaxed){
        let now=Instant::now();
        let due=schedule.due(now);
        let mut fresh_nodes=if due {pnp_nodes()}else{None};
        if let Some(found)=&fresh_nodes {nodes=found.clone();}
        if let Some(found)=discover(&state.targets(),&nodes,&stop){
            state.discover(found,now);let targets=state.targets();
            let ids:Vec<_>=targets.iter().map(|t|(t.endpoint.id.clone(),t.session)).collect();
            if schedule.begin_read(now,ids) {
                // 新连接需要重新查询，不能把上轮缓存重新打上当前时间。
                if !due {fresh_nodes=pnp_nodes();}
                if let Some(fresh)=fresh_nodes{
                    for sample in windows_samples(&targets,&fresh,Instant::now()) {state.ingest(sample);}
                    nodes=fresh;
                }
                let _=jobs.try_send(targets);
            }
            while let Ok(reading)=readings.try_recv(){state.ingest(reading);}
            if matches!(tx.try_send(state.devices(Instant::now())),Err(mpsc::TrySendError::Disconnected(_))){break;}
        }
        for _ in 0..20 {if stop.load(Ordering::Relaxed){break;}std::thread::sleep(Duration::from_millis(100));}
    }
    stop.store(true,Ordering::Relaxed);drop(jobs);if let Some(worker)=worker{let _=worker.join();}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bluetooth_poll_interval_and_reconnect() {
        let now=Instant::now();
        let ids=vec![("le".to_string(),1)];
        let mut schedule=PollSchedule::default();
        assert!(schedule.begin_read(now,ids.clone()));
        assert!(!schedule.begin_read(now+Duration::from_secs(59),ids.clone()));
        let next=now+Duration::from_secs(60);
        assert!(schedule.begin_read(next,ids));
        assert!(schedule.begin_read(next,vec![("le".into(),2)]));
        assert!(schedule.begin_read(next,vec![]));
    }

    #[test]
    fn bluetooth_poll_interval_advances_when_gatt_queue_is_full() {
        let now=Instant::now();
        let ids=vec![("le".to_string(),1)];
        let (jobs,_requests)=mpsc::sync_channel(1);
        jobs.try_send(()).unwrap();

        let mut schedule=PollSchedule::default();
        assert!(schedule.begin_read(now,ids.clone()));
        assert!(matches!(jobs.try_send(()),Err(mpsc::TrySendError::Full(()))));
        assert!(!schedule.begin_read(now+Duration::from_secs(2),ids.clone()));
        assert!(schedule.begin_read(now+Duration::from_secs(60),ids));
    }

    #[test]
    fn bluetooth_windows_nodes_merge_before_gatt_fallback() {
        let now=Instant::now();
        let mut state=State::default();
        let ep=|id:&str,addr:&str|Endpoint{id:id.into(),name:id.into(),address:Some(addr.into()),
            kind:DeviceKind::Other,container:Some("group".into()),ble:true};
        state.discover(vec![ep("a","001122334455"),ep("b","001122334456")],now);
        let targets=state.targets();
        let node=|addr:&str,battery|PnpNode{address:addr.into(),battery,container:None};
        let nodes=[node("001122334455",None),node("001122334456",Some(32)),node("001122334455",Some(70)),node("001122334457",Some(1))];
        let mut samples=windows_samples(&targets,&nodes,now);
        assert_eq!(samples.len(),1);
        let mut sample=samples.pop().unwrap();assert_eq!(sample.reading.percent,32);
        state.ingest(sample.clone());
        sample.reading=BatteryReading::new(35,BatterySource::Gatt,now,Some(true));
        state.ingest(sample);
        assert_eq!(state.devices(now)[0].battery,Some(35));
        let later=now+FRESH_FOR+Duration::from_secs(1);
        for s in windows_samples(&targets,&[node("001122334455",Some(30))],later) {state.ingest(s);}
        assert_eq!(state.devices(later)[0].battery,Some(30));
    }

    #[test]
    #[ignore="按需读取本机已连接设备，等待两个来源返回"]
    fn bluetooth_live_sources(){
        let service=crate::bt::spawn();let start=std::time::Instant::now();let mut snapshots=0;
        while start.elapsed()<std::time::Duration::from_secs(20){
            if let Ok(devices)=service.rx.recv_timeout(std::time::Duration::from_secs(1)){
                snapshots+=1;
                if snapshots==1||start.elapsed().as_secs()>=18 {
                    for device in &devices {println!("{} | connected={} | battery={:?} | source={:?} | charging={:?}",device.name,device.connected,device.battery,device.reading.as_ref().map(|r|r.source),device.reading.as_ref().and_then(|r|r.charging));}
                    println!("snapshot devices={}",devices.len());
                }
            }
        }
        assert!(snapshots>0,"Windows 设备枚举应返回完整快照");
    }
}
