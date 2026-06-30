import { invoke } from '@tauri-apps/api/core';
import type { VmView, CreateVmRequest } from './types';

export const listVms = () => invoke<VmView[]>('list_vms');
export const createVm = (req: CreateVmRequest) => invoke<VmView>('create_vm', { req });
export const startVm = (id: string) => invoke<VmView>('start_vm', { id });
export const stopVm = (id: string) => invoke<void>('stop_vm', { id });
export const pauseVm = (id: string) => invoke<void>('pause_vm', { id });
export const resumeVm = (id: string) => invoke<void>('resume_vm', { id });
export const deleteVm = (id: string) => invoke<void>('delete_vm', { id });
export const openConsole = (id: string) => invoke<number[]>('open_console', { id });
export const consoleInput = (id: string, data: number[]) =>
  invoke<void>('console_input', { id, data });
export const closeConsole = (id: string) => invoke<void>('close_console', { id });
export const consoleLogPath = (id: string) => invoke<string>('console_log_path', { id });
