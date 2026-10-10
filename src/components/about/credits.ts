/** Enlaces del desarrollador y de la comunidad (los mismos que en HiveShock). */
export const LINKS = {
  developer: "Yafel",
  website: "https://yafel.dev",
  instagram: "https://www.instagram.com/yaafel/",
  instagramHandle: "@yaafel",
  github: "https://github.com/Lexharden/HiveBuzz",
  discord: "https://discord.gg/QTdQffuZF3",
  youtube: "https://www.youtube.com/@HiveShock",
  kofi: "https://ko-fi.com/yafel",
} as const;

export interface CreditMember {
  name: string;
  /** Usuario de TikTok sin @ (opcional): el nombre se vuelve un enlace a su perfil. */
  tiktok?: string;
}

export interface CreditCategory {
  /** Clave de i18n en `about.team.cat.<key>`. */
  key: "dev" | "admins" | "support" | "testers";
  icon: string;
  members: CreditMember[];
}

/** Equipo de «Acerca de». Añadir a alguien es añadirlo aquí; las categorías vacías no se muestran. */
export const CREDITS: CreditCategory[] = [
  { key: "dev", icon: "🛠️", members: [{ name: "Yafel" }] },
  { key: "admins", icon: "🛡️", members: [] },
  { key: "support", icon: "💬", members: [] },
  { key: "testers", icon: "🧪", members: [] },
];
