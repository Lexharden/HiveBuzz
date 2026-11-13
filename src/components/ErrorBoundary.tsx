import { Component, type ErrorInfo, type ReactNode } from "react";
import i18n from "../i18n";
import { systemApi } from "../lib/system";

interface Props {
  children: ReactNode;
}

interface State {
  error: Error | null;
}

/**
 * Atrapa los errores de render de una pantalla: sin esto, uno solo deja la ventana entera en blanco en pleno directo.
 * El error se anota en `hivebuzz.log` (para soporte) y la pantalla ofrece reintentar; el resto de la app sigue igual.
 * Se monta con `key` = página, así que cambiar de pantalla también lo reinicia.
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    const stack = [error.stack, info.componentStack].filter(Boolean).join("\n");
    void systemApi.reportUiError(error.message, stack).catch(() => undefined);
  }

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <div role="alert" className="rounded-lg border border-rose-500/40 bg-rose-500/10 p-4 text-sm text-rose-200">
        <p className="font-semibold">{i18n.t("ui.crashTitle")}</p>
        <p className="mt-1 text-rose-300/80">{i18n.t("ui.crashText")}</p>
        <pre className="mt-2 max-h-32 overflow-auto whitespace-pre-wrap text-xs text-rose-300/70">{error.message}</pre>
        <button
          type="button"
          onClick={() => this.setState({ error: null })}
          className="mt-3 rounded-md border border-rose-400/50 px-3 py-1 text-xs hover:bg-rose-500/20"
        >
          {i18n.t("ui.crashRetry")}
        </button>
      </div>
    );
  }
}
