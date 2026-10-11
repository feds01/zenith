import { Button } from "@acme/ui/button";
import { CampaignRoute } from "@/routes/campaigns.$id";
import { HomeRoute } from "@/routes/home";

export function App() {
  return (
    <main>
      <HomeRoute />
      <CampaignRoute id="launch" />
      <Button label="Sign out" />
    </main>
  );
}
