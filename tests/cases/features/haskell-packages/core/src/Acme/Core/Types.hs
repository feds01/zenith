module Acme.Core.Types
  ( Id (..)
  , Config
  ) where

import {-# SOURCE #-} Acme.Core (Config)
-- zenith-ignore core-is-the-base: the routes move into the core next
import Acme.Api.Routes (Route)
import qualified Acme.Api as Api

newtype Id = Id Int
