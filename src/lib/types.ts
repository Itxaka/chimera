export type VmStatus = 'creating' | 'running' | 'paused' | 'stopped' | 'failed';

export interface DiskConfig { path: string; readonly: boolean; }
export interface NetConfig { bridge: string; }
export type BootConfig = { kind: 'firmware'; firmware: string };

export interface VmDefinition {
  id: string;
  name: string;
  vcpus: number;
  memory_mib: number;
  disks: DiskConfig[];
  net: NetConfig;
  boot: BootConfig;
  created_at: string;
}

export interface VmRuntime {
  pid: number | null;
  socket: string;
  tap: string | null;
  status: VmStatus;
  last_error: string | null;
}

export interface VmView { definition: VmDefinition; runtime: VmRuntime; }

export interface CreateVmRequest {
  name: string;
  vcpus: number;
  memory_mib: number;
  disk_path: string;
  firmware_path: string;
  bridge: string;
}
