import { createRoot } from 'react-dom/client';
import { installTauriApiBridge } from './desktopApi.js';
import App from './App.jsx';
import './styles/index.css';

installTauriApiBridge();

const root = document.getElementById('root');
createRoot(root).render(<App />);
