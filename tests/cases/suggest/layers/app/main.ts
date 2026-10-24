import { order } from "../domain/order";
import { connect } from "../infra/db";

export const main = () => [order, connect];
