use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use sysinfo::{Disks, Pid, System};
use tokio::time::interval;

pub struct ResourceManager {
    system: Arc<Mutex<System>>,
    check_interval: Duration,
    data_dir: PathBuf,
    pid: Pid,
}

impl ResourceManager {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            system: Arc::new(Mutex::new(System::new_all())),
            check_interval: Duration::from_secs(10),
            data_dir,
            pid: Pid::from(std::process::id() as usize),
        }
    }

    pub async fn start_monitoring(&self) {
        let system = self.system.clone();
        let mut interval = interval(self.check_interval);
        let pid = self.pid;
        let data_dir = self.data_dir.clone();

        tokio::spawn(async move {
            loop {
                interval.tick().await;

                // Scope for system lock
                {
                    let mut sys = system.lock().unwrap();
                    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);

                    if let Some(process) = sys.process(pid) {
                        // Check CPU (Process usage / Total Cores)
                        let num_cpus = sys.cpus().len() as f32;
                        let process_cpu = process.cpu_usage(); // 0..100 * num_cpus
                        let cpu_percent = process_cpu / num_cpus;

                        if cpu_percent > 10.0 {
                            tracing::warn!("App High CPU usage: {:.2}% (Limit: 10%)", cpu_percent);
                        }

                        // Check Memory (Process Memory / Total Memory)
                        let process_memory = process.memory(); // bytes
                        let total_memory = sys.total_memory();
                        let memory_percent = (process_memory as f64 / total_memory as f64) * 100.0;

                        if memory_percent > 10.0 {
                            tracing::warn!(
                                "App High Memory usage: {:.2}% (Limit: 10%)",
                                memory_percent
                            );
                        }
                    }
                }

                // Check Disk (App Data Dir Size / Total Disk Space)
                // We calculate the size of the data_dir
                let app_disk_usage = get_dir_size(&data_dir).unwrap_or(0);

                let disks = Disks::new_with_refreshed_list();
                // Find the disk that contains the data_dir, or default to first
                if let Some(disk) = disks.list().first() {
                    let total_space = disk.total_space();
                    let disk_percent = (app_disk_usage as f64 / total_space as f64) * 100.0;

                    if disk_percent > 10.0 {
                        tracing::warn!("App High Disk usage: {:.2}% (Limit: 10%)", disk_percent);
                    }
                }
            }
        });
    }
}

fn get_dir_size(path: &std::path::Path) -> std::io::Result<u64> {
    let mut total_size = 0;
    if path.exists() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let metadata = entry.metadata()?;
            if metadata.is_dir() {
                total_size += get_dir_size(&entry.path())?;
            } else {
                total_size += metadata.len();
            }
        }
    }
    Ok(total_size)
}
