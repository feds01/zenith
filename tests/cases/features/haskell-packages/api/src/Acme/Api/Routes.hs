module Acme.Api.Routes (Route, route) where

import Acme.Core (Id (..))

newtype Route = Route Int

route :: Id -> Route
route (Id n) = Route n
