import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { BrowserRouter as Router, Navigate, Route, Routes } from "react-router-dom";
import "@fontsource-variable/geist";
import "@fontsource-variable/geist-mono";
import "./styles.css";

import { Shell } from "./components/Shell";
import { ToastProvider } from "./components/Toast";
import { Inicio } from "./pages/Inicio";
import { Lotes } from "./pages/Lotes";
import { Biblioteca } from "./pages/Biblioteca";
import { Modelos } from "./pages/Modelos";
import { Configuracoes } from "./pages/Configuracoes";
import { IA } from "./pages/IA";
import { Clipes } from "./pages/Clipes";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ToastProvider>
      <Router>
        <Routes>
          <Route element={<Shell />}>
            <Route index element={<Inicio />} />
            <Route path="lotes" element={<Lotes />} />
            <Route path="clipes" element={<Clipes />} />
            <Route path="biblioteca" element={<Biblioteca />} />
            <Route path="modelos" element={<Modelos />} />
            <Route path="ia" element={<IA />} />
            <Route path="configuracoes" element={<Configuracoes />} />
            <Route path="*" element={<Navigate to="/" replace />} />
          </Route>
        </Routes>
      </Router>
    </ToastProvider>
  </StrictMode>,
);
