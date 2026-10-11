import { blueprintFor } from "../blueprints";
import { baseStrategy } from "../shared";

export const adsStrategy = { ...baseStrategy, channel: "ads", blueprint: blueprintFor("ads") };
