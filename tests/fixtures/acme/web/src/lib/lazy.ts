export async function loadChart() {
  return import(`./lazy-chart`);
}

export async function loadLocale(name: string) {
  return import(name);
}
