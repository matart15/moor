import { Outlet } from "react-router-dom";
import { Sidebar } from "./Sidebar";
import { Header } from "./Header";
import { useSettings } from "@/hooks/useSettings";
import { useTheme } from "@/hooks/useTheme";

export function AppShell() {
  const { settings, isError, isFetched } = useSettings();
  useTheme(isFetched && !isError ? settings.appearance.theme : null);

  return (
    <div className="flex h-screen bg-cursor-cream overflow-hidden">
      <Sidebar />
      <div className="flex-1 flex flex-col min-w-0">
        <Header />
        <main className="flex-1 overflow-auto p-4">
          <Outlet />
        </main>
      </div>
    </div>
  );
}
