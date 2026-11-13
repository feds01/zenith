import { forwardRef } from "react";
import { useLayoutEffect } from "react";

export const useMeasure = () => useLayoutEffect(() => {});
export const Measured = forwardRef(() => null);
