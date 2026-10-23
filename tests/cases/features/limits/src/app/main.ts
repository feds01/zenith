import React from "react";
import { log } from "../lib/log";
import { format } from "../lib/format";
import { Button } from "../ui/button";

export const main = () => log(format(String(Button)), React);
