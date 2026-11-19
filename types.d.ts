interface Window {
  __TAURI__: {
    core: {
      invoke(command: string, arguments: Record<string, unknown>): Promise<any>;
    };
  };
}
