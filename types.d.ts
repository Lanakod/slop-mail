interface Window {
  __composeContext?: {profile:string;slot:string;to:string;subject:string;body:string};
  __TAURI__: {
    window: {
      getCurrentWindow(): {
        onCloseRequested(
          handler: (event: { preventDefault(): void }) => Promise<void>,
        ): Promise<unknown>;
        minimize(): Promise<void>;
        toggleMaximize(): Promise<void>;
        close(): Promise<void>;
        destroy(): Promise<void>;
      };
    };
    webview: {getCurrentWebview(): {onDragDropEvent(handler:(e:any)=>void): Promise<unknown>}};
    event: {
      emitTo(target:string,event:string,payload:any):Promise<unknown>;
      listen(
        name: string,
        handler: (event: { payload: any }) => void,
      ): Promise<unknown>;
    };
    core: {
      invoke(command: string, arguments: Record<string, unknown>): Promise<any>;
    };
  };
}
