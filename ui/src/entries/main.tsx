/**
 * Single UI entry — routes between MainApp / EmbedApp / ConsumerApp based on:
 *   URL path /embed          → EmbedApp
 *   URL ?mode=compact (shim) → EmbedApp
 *   currentPage === 'consumer' (from user_info) → ConsumerApp
 *   otherwise                → MainApp
 *
 * The `view` signal (main/embed/consumer) is computed here and pushed to the
 * server whenever it changes — each App only needs to notify on session /
 * project changes within its own view.
 */
import React, { useEffect } from 'react';
import ReactDOM from 'react-dom/client';
import { MainApp } from '../apps/MainApp';
import { EmbedApp } from '../apps/EmbedApp';
import { ConsumerApp } from '../apps/ConsumerApp';
import { ErrorBoundary } from '../components/ErrorBoundary';
import { useUiStore } from '../stores/uiStore';
import { sendViewContext } from '../hooks/useTransport';
import '../index.css';
import { installFetchProxy } from '../lib/fetchProxy';

installFetchProxy();

const path = window.location.pathname;
const urlParams = new URLSearchParams(window.location.search);
const isEmbedPath = path === '/embed' || path.startsWith('/embed/')
  || urlParams.get('mode') === 'compact';

type View = 'main' | 'embed' | 'consumer';

const Root: React.FC = () => {
  const currentPage = useUiStore((s) => s.currentPage);

  const view: View = isEmbedPath
    ? 'embed'
    : currentPage === 'consumer'
      ? 'consumer'
      : 'main';

  // Keep the global in sync, then push the view to the server whenever it
  // changes (e.g. main → consumer once user_info arrives). sendViewContext
  // is a no-op if the transport isn't connected yet; onReconnect will send
  // the initial value once it is.
  useEffect(() => {
    (window as { __LINGGEN_VIEW__?: View }).__LINGGEN_VIEW__ = view;
    sendViewContext();
  }, [view]);

  if (view === 'embed') return <EmbedApp />;
  if (view === 'consumer') return <ConsumerApp />;
  return <MainApp />;
};

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <ErrorBoundary>
      <Root />
    </ErrorBoundary>
  </React.StrictMode>
);
