interface Window {
  __TAURI__: {
    window: {
      getCurrentWindow(): {
        onCloseRequested(
          handler: (event: { preventDefault(): void }) => Promise<void>,
        ): Promise<unknown>;
        destroy(): Promise<void>;
      };
    };
    core: {
      invoke(command: string, arguments: Record<string, unknown>): Promise<any>;
    };
  };
}
